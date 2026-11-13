//! A Cranelift JIT for `Program`, bit-identical to the interpreter (`Program::eval_n`).
//!
//! How the two stay identical: arithmetic that IEEE 754 fixes exactly (`+ - * /`, negation, `abs`, `sqrt`, `floor`,
//! `ceil`, `trunc`, comparisons, selects, the integer hash) is emitted as native instructions; everything else
//! (the `libm` functions, `round`, `noise`) is a call to a thunk that runs the very same Rust function the
//! interpreter runs (`un_apply`, `bin_apply`, `noise_apply`). Nothing is ever fused (no FMA) or reassociated. A NaN
//! result is a NaN in both, but its sign and payload bits are not promised to match.
//!
//! The generated function does what the interpreter does, but fused: nodes that depend on no per-point input are
//! computed once before the loop, and the rest are computed per point in registers.
use crate::eval::EvalError;
use crate::ir::*;
use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlagsData, Signature, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JitError(pub String);

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the JIT could not compile this program: {}", f_msg(self))
    }
}
fn f_msg(e: &JitError) -> &str {
    &e.0
}

type Kernel =
    unsafe extern "C" fn(usize, *const *const f64, *const f64, *const f64, *const *mut f64);

/// A program compiled to machine code. Same results as `Program::eval_n`, see the module docs.
pub struct Compiled {
    prog: Program,
    module: Option<JITModule>,
    kernel: Kernel,
}

impl Drop for Compiled {
    fn drop(&mut self) {
        if let Some(m) = self.module.take() {
            // SAFETY: the kernel is never called after this, and nothing else points into the module's memory.
            unsafe { m.free_memory() };
        }
    }
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Compiled({} nodes)", self.prog.nodes.len())
    }
}

// ---- the thunks: the same Rust functions as the interpreter's -------------------------------------------------

macro_rules! un_thunks {
    ($($name:ident = $op:ident),* $(,)?) => {
        $(extern "C" fn $name(x: f64) -> f64 { un_apply(Un::$op, x) })*
        fn un_thunk(op: Un) -> Option<usize> {
            match op { $(Un::$op => Some($name as *const () as usize),)* _ => None }
        }
    };
}
un_thunks! {
    t_sin = Sin, t_cos = Cos, t_tan = Tan, t_asin = Asin, t_acos = Acos, t_atan = Atan,
    t_sinh = Sinh, t_cosh = Cosh, t_tanh = Tanh, t_exp = Exp, t_log = Log, t_log2 = Log2,
    t_log10 = Log10, t_round = Round,
}
macro_rules! bin_thunks {
    ($($name:ident = $op:ident),* $(,)?) => {
        $(extern "C" fn $name(a: f64, b: f64) -> f64 { bin_apply(Bin::$op, a, b) })*
        fn bin_thunk(op: Bin) -> Option<usize> {
            match op { $(Bin::$op => Some($name as *const () as usize),)* _ => None }
        }
    };
}
bin_thunks! { t_rem = Rem, t_pow = Pow, t_atan2 = Atan2 }
extern "C" fn t_noise(dims: u32, seed: u32, x: f64, y: f64, z: f64) -> f64 {
    noise_apply(dims as u8, seed, x, y, z)
}

// ---- code generation -------------------------------------------------------------------------------------------

struct Emit<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    sig1: cranelift_codegen::ir::SigRef,
    sig2: cranelift_codegen::ir::SigRef,
    sig_noise: cranelift_codegen::ir::SigRef,
}

impl Emit<'_, '_> {
    fn flag(&mut self, cc: FloatCC, a: Value, b: Value) -> Value {
        let c = self.b.ins().fcmp(cc, a, b);
        let one = self.b.ins().f64const(1.0);
        let zero = self.b.ins().f64const(0.0);
        self.b.ins().select(c, one, zero)
    }
    fn call1(&mut self, f: usize, x: Value) -> Value {
        let p = self.b.ins().iconst(types::I64, f as i64);
        let call = self.b.ins().call_indirect(self.sig1, p, &[x]);
        self.b.inst_results(call)[0]
    }
    fn call2(&mut self, f: usize, x: Value, y: Value) -> Value {
        let p = self.b.ins().iconst(types::I64, f as i64);
        let call = self.b.ins().call_indirect(self.sig2, p, &[x, y]);
        self.b.inst_results(call)[0]
    }

    fn un(&mut self, op: Un, a: Value) -> Value {
        match op {
            Un::Neg => self.b.ins().fneg(a),
            Un::Abs => self.b.ins().fabs(a),
            Un::Sqrt => self.b.ins().sqrt(a),
            Un::Floor => self.b.ins().floor(a),
            Un::Ceil => self.b.ins().ceil(a),
            Un::Trunc => self.b.ins().trunc(a),
            Un::Fract => {
                let f = self.b.ins().floor(a);
                self.b.ins().fsub(a, f)
            }
            Un::Sign => {
                let zero = self.b.ins().f64const(0.0);
                let (one, mone) = (self.b.ins().f64const(1.0), self.b.ins().f64const(-1.0));
                let pos = self.b.ins().fcmp(FloatCC::GreaterThan, a, zero);
                let neg = self.b.ins().fcmp(FloatCC::LessThan, a, zero);
                let r = self.b.ins().select(neg, mone, zero);
                self.b.ins().select(pos, one, r)
            }
            Un::Not => {
                let zero = self.b.ins().f64const(0.0);
                self.flag(FloatCC::Equal, a, zero)
            }
            Un::Hash => {
                let mut z = self.b.ins().bitcast(types::I64, MemFlagsData::new(), a);
                let k = self
                    .b
                    .ins()
                    .iconst(types::I64, 0x9E37_79B9_7F4A_7C15u64 as i64);
                z = self.b.ins().iadd(z, k);
                let s = self.b.ins().ushr_imm_u(z, 30);
                z = self.b.ins().bxor(z, s);
                let k = self
                    .b
                    .ins()
                    .iconst(types::I64, 0xBF58_476D_1CE4_E5B9u64 as i64);
                z = self.b.ins().imul(z, k);
                let s = self.b.ins().ushr_imm_u(z, 27);
                z = self.b.ins().bxor(z, s);
                let k = self
                    .b
                    .ins()
                    .iconst(types::I64, 0x94D0_49BB_1331_11EBu64 as i64);
                z = self.b.ins().imul(z, k);
                let s = self.b.ins().ushr_imm_u(z, 31);
                z = self.b.ins().bxor(z, s);
                let top = self.b.ins().ushr_imm_u(z, 11);
                let f = self.b.ins().fcvt_from_uint(types::F64, top);
                let d = self.b.ins().f64const((1u64 << 53) as f64);
                self.b.ins().fdiv(f, d)
            }
            op => self.call1(un_thunk(op).expect("a thunk for every other op"), a),
        }
    }

    fn bin(&mut self, op: Bin, a: Value, b: Value) -> Value {
        match op {
            Bin::Add => self.b.ins().fadd(a, b),
            Bin::Sub => self.b.ins().fsub(a, b),
            Bin::Mul => self.b.ins().fmul(a, b),
            Bin::Div => self.b.ins().fdiv(a, b),
            // if a.is_nan() || b < a { b } else { a }
            Bin::Min | Bin::Max => {
                let nan = self.b.ins().fcmp(FloatCC::Unordered, a, a);
                let better = if op == Bin::Min {
                    self.b.ins().fcmp(FloatCC::LessThan, b, a)
                } else {
                    self.b.ins().fcmp(FloatCC::GreaterThan, b, a)
                };
                let c = self.b.ins().bor(nan, better);
                self.b.ins().select(c, b, a)
            }
            Bin::Lt => self.flag(FloatCC::LessThan, a, b),
            Bin::Le => self.flag(FloatCC::LessThanOrEqual, a, b),
            Bin::Gt => self.flag(FloatCC::GreaterThan, a, b),
            Bin::Ge => self.flag(FloatCC::GreaterThanOrEqual, a, b),
            Bin::Eq => self.flag(FloatCC::Equal, a, b),
            Bin::Ne => self.flag(FloatCC::NotEqual, a, b),
            Bin::And | Bin::Or => {
                let zero = self.b.ins().f64const(0.0);
                let x = self.b.ins().fcmp(FloatCC::NotEqual, a, zero);
                let y = self.b.ins().fcmp(FloatCC::NotEqual, b, zero);
                let c = if op == Bin::And {
                    self.b.ins().band(x, y)
                } else {
                    self.b.ins().bor(x, y)
                };
                let (one, zero) = (self.b.ins().f64const(1.0), self.b.ins().f64const(0.0));
                self.b.ins().select(c, one, zero)
            }
            op => self.call2(bin_thunk(op).expect("a thunk for every other op"), a, b),
        }
    }

    fn select(&mut self, c: Value, a: Value, b: Value) -> Value {
        let zero = self.b.ins().f64const(0.0);
        let cond = self.b.ins().fcmp(FloatCC::NotEqual, c, zero);
        self.b.ins().select(cond, a, b)
    }

    fn noise(&mut self, dims: u8, seed: u32, args: [Value; 3]) -> Value {
        let p = self
            .b
            .ins()
            .iconst(types::I64, t_noise as *const () as usize as i64);
        let d = self.b.ins().iconst(types::I32, dims as i64);
        let s = self.b.ins().iconst(types::I32, seed as i64);
        let call =
            self.b
                .ins()
                .call_indirect(self.sig_noise, p, &[d, s, args[0], args[1], args[2]]);
        self.b.inst_results(call)[0]
    }
}

impl Program {
    /// Compile to machine code. Fails (and the caller should use the interpreter) if the host CPU or the
    /// operating system does not allow it.
    pub fn compile(&self) -> Result<Compiled, JitError> {
        let e = |what: &str, err: &dyn std::fmt::Display| JitError(format!("{what}: {err}"));
        let mut flags = settings::builder();
        flags
            .set("opt_level", "speed")
            .map_err(|x| e("settings", &x))?;
        flags
            .set("is_pic", "false")
            .map_err(|x| e("settings", &x))?;
        let isa = cranelift_native::builder()
            .map_err(|x| e("host", &x))?
            .finish(settings::Flags::new(flags))
            .map_err(|x| e("isa", &x))?;
        let call_conv = isa.default_call_conv();
        let frontend = isa.frontend_config();
        let ptr = isa.pointer_type();
        let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));

        let mut sig = module.make_signature();
        for _ in 0..5 {
            sig.params.push(AbiParam::new(ptr));
        }
        let func = module
            .declare_function("kernel", Linkage::Local, &sig)
            .map_err(|x| e("declare", &x))?;
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fb_ctx = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let mk = |ints: &[bool]| {
                let mut s = Signature::new(call_conv);
                for int in ints {
                    s.params
                        .push(AbiParam::new(if *int { types::I32 } else { types::F64 }));
                }
                s.returns.push(AbiParam::new(types::F64));
                s
            };
            let sig1 = b.import_signature(mk(&[false]));
            let sig2 = b.import_signature(mk(&[false, false]));
            let sig_noise = b.import_signature(mk(&[true, true, false, false, false]));

            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            let header = b.create_block();
            let body = b.create_block();
            let exit = b.create_block();
            b.switch_to_block(entry);
            let (n, points, frame, params, outs) = {
                let p = b.block_params(entry);
                (p[0], p[1], p[2], p[3], p[4])
            };
            let flags = MemFlagsData::trusted();
            let slots = self.input_slots();
            let varying = self.varying();
            let mut vals: Vec<Option<Value>> = vec![None; self.nodes.len()];
            // the point-input column pointers and the output pointers
            let n_point = slots.iter().filter(|s| s.0 == Rate::Point).count();
            let cols: Vec<Value> = (0..n_point)
                .map(|k| b.ins().load(ptr, flags, points, (k * 8) as i32))
                .collect();
            let out_ptrs: Vec<Value> = (0..self.outputs.len())
                .map(|k| b.ins().load(ptr, flags, outs, (k * 8) as i32))
                .collect();

            // uniform nodes, once
            for (idx, node) in self.nodes.iter().enumerate() {
                if varying[idx] {
                    continue;
                }
                let mut em = Emit {
                    b: &mut b,
                    sig1,
                    sig2,
                    sig_noise,
                };
                vals[idx] = Some(uniform_or_point(
                    &mut em, node, &vals, &slots, frame, params, None, flags, ptr,
                ));
            }
            let i = b.declare_var(ptr);
            let zero = b.ins().iconst(ptr, 0);
            b.def_var(i, zero);
            b.ins().jump(header, &[]);

            b.switch_to_block(header);
            let iv = b.use_var(i);
            let more = b.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::UnsignedLessThan,
                iv,
                n,
            );
            b.ins().brif(more, body, &[], exit, &[]);

            b.switch_to_block(body);
            let iv = b.use_var(i);
            let off = b.ins().ishl_imm_u(iv, 3);
            let mut pv: Vec<Option<Value>> = vals.clone();
            for (idx, node) in self.nodes.iter().enumerate() {
                if !varying[idx] {
                    continue;
                }
                let mut em = Emit {
                    b: &mut b,
                    sig1,
                    sig2,
                    sig_noise,
                };
                pv[idx] = Some(uniform_or_point(
                    &mut em,
                    node,
                    &pv,
                    &slots,
                    frame,
                    params,
                    Some((&cols, off)),
                    flags,
                    ptr,
                ));
            }
            for (k, o) in self.outputs.iter().enumerate() {
                let a = b.ins().iadd(out_ptrs[k], off);
                b.ins()
                    .store(flags, pv[*o as usize].expect("computed"), a, 0);
            }
            let one = b.ins().iconst(ptr, 1);
            let next = b.ins().iadd(iv, one);
            b.def_var(i, next);
            b.ins().jump(header, &[]);

            b.switch_to_block(exit);
            b.ins().return_(&[]);
            b.seal_all_blocks();
            b.finalize(frontend);
        }
        module
            .define_function(func, &mut ctx)
            .map_err(|x| e("define", &x))?;
        module.clear_context(&mut ctx);
        module
            .finalize_definitions()
            .map_err(|x| e("finalize", &x))?;
        let code = module.get_finalized_function(func);
        // SAFETY: the function was generated with exactly this signature (five pointer-sized arguments, no result)
        let kernel: Kernel = unsafe { std::mem::transmute(code) };
        Ok(Compiled {
            prog: self.clone(),
            module: Some(module),
            kernel,
        })
    }
}

/// One node's value: a point input is loaded from its column at `off` (when given), everything else is computed from
/// the already known values of its operands.
#[allow(clippy::too_many_arguments)]
fn uniform_or_point(
    em: &mut Emit,
    node: &Node,
    vals: &[Option<Value>],
    slots: &[(Rate, usize)],
    frame: Value,
    params: Value,
    point: Option<(&[Value], Value)>,
    flags: MemFlagsData,
    ptr: cranelift_codegen::ir::Type,
) -> Value {
    let get = |id: Id| vals[id as usize].expect("operands come first");
    match *node {
        Node::Const(bits) => em.b.ins().f64const(f64::from_bits(bits)),
        Node::Param(k) => em.b.ins().load(types::F64, flags, params, (k * 8) as i32),
        Node::Input(k) => match slots[k as usize] {
            (Rate::Frame, s) => em.b.ins().load(types::F64, flags, frame, (s * 8) as i32),
            (Rate::Point, s) => {
                let (cols, off) = point.expect("point inputs are varying");
                let a = em.b.ins().iadd(cols[s], off);
                let _ = ptr;
                em.b.ins().load(types::F64, flags, a, 0)
            }
        },
        Node::Un(op, a) => em.un(op, get(a)),
        Node::Bin(op, a, b) => em.bin(op, get(a), get(b)),
        Node::Select(c, a, b) => em.select(get(c), get(a), get(b)),
        Node::Noise { dims, seed, args } => {
            em.noise(dims, seed, [get(args[0]), get(args[1]), get(args[2])])
        }
    }
}

impl Compiled {
    /// Same contract as `Program::eval_n`.
    pub fn eval_n(
        &self,
        n: usize,
        point: &[&[f64]],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<Vec<f64>>, EvalError> {
        self.prog.slots(n, point, frame, params)?;
        let mut out: Vec<Vec<f64>> = vec![vec![0.0; n]; self.prog.outputs.len()];
        let cols: Vec<*const f64> = point.iter().map(|p| p.as_ptr()).collect();
        let outs: Vec<*mut f64> = out.iter_mut().map(|o| o.as_mut_ptr()).collect();
        // SAFETY: `slots` checked that every column has `n` numbers and the counts of inputs and parameters;
        // every output has `n` numbers; the kernel reads and writes nothing else.
        unsafe {
            (self.kernel)(
                n,
                cols.as_ptr(),
                frame.as_ptr(),
                params.as_ptr(),
                outs.as_ptr(),
            )
        };
        Ok(out)
    }
    pub fn eval(
        &self,
        point: &[&[f64]],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<Vec<f64>>, EvalError> {
        self.eval_n(point.first().map_or(1, |p| p.len()), point, frame, params)
    }
    pub fn program(&self) -> &Program {
        &self.prog
    }
}
