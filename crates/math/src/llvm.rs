//! A spike: the same kernel as `jit.rs`, built with LLVM 18 (inkwell) and run through the O3 pipeline, so the loop
//! vectorizer may use several lanes. Results must stay bit-identical to the interpreter: only IEEE-exact instructions
//! are emitted natively, no fast-math flags are set, and everything else calls the same thunks as the Cranelift backend.
use crate::eval::EvalError;
use crate::ir::*;
use crate::thunks::{bin_thunk, noise_thunk_addr, un_thunk};
use inkwell::builder::Builder as LB;
use inkwell::context::Context;
use inkwell::intrinsics::Intrinsic;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::values::{BasicMetadataValueEnum, FloatValue, IntValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlvmError(pub String);

impl std::fmt::Display for LlvmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LLVM could not compile this program: {}", self.0)
    }
}

type Kernel =
    unsafe extern "C" fn(usize, *const *const f64, *const f64, *const f64, *const *mut f64);

/// A program compiled by LLVM. Same results as `Program::eval_n`.
pub struct LlvmCompiled {
    prog: Program,
    kernel: Kernel,
    // keep the engine (and with it the machine code) alive; the context is leaked for the lifetime of the process
    _engine: inkwell::execution_engine::ExecutionEngine<'static>,
}

struct Em<'a> {
    ctx: &'static Context,
    b: &'a LB<'static>,
    m: &'a Module<'static>,
}

impl<'a> Em<'a> {
    fn f64t(&self) -> inkwell::types::FloatType<'static> {
        self.ctx.f64_type()
    }
    fn intrinsic1(&self, name: &str, x: FloatValue<'static>) -> FloatValue<'static> {
        let i = Intrinsic::find(name).expect("an LLVM intrinsic");
        let d = i
            .get_declaration(self.m, &[self.f64t().into()])
            .expect("a declaration");
        self.b
            .build_call(d, &[x.into()], "")
            .unwrap()
            .try_as_basic_value()
            .unwrap_basic()
            .into_float_value()
    }
    fn flag(
        &self,
        p: FloatPredicate,
        a: FloatValue<'static>,
        b: FloatValue<'static>,
    ) -> FloatValue<'static> {
        let c = self.b.build_float_compare(p, a, b, "").unwrap();
        self.b
            .build_select(
                c,
                self.f64t().const_float(1.0),
                self.f64t().const_float(0.0),
                "",
            )
            .unwrap()
            .into_float_value()
    }
    fn call_addr(
        &self,
        addr: usize,
        params: &[inkwell::types::BasicMetadataTypeEnum<'static>],
        args: &[BasicMetadataValueEnum<'static>],
    ) -> FloatValue<'static> {
        let ft = self.f64t().fn_type(params, false);
        let p = self
            .ctx
            .i64_type()
            .const_int(addr as u64, false)
            .const_to_pointer(self.ctx.ptr_type(AddressSpace::default()));
        self.b
            .build_indirect_call(ft, p, args, "")
            .unwrap()
            .try_as_basic_value()
            .unwrap_basic()
            .into_float_value()
    }

    fn un(&self, op: Un, a: FloatValue<'static>) -> FloatValue<'static> {
        let f = self.f64t();
        match op {
            Un::Neg => self.b.build_float_neg(a, "").unwrap(),
            Un::Abs => self.intrinsic1("llvm.fabs", a),
            Un::Sqrt => self.intrinsic1("llvm.sqrt", a),
            Un::Floor => self.intrinsic1("llvm.floor", a),
            Un::Ceil => self.intrinsic1("llvm.ceil", a),
            Un::Trunc => self.intrinsic1("llvm.trunc", a),
            Un::Fract => {
                let fl = self.intrinsic1("llvm.floor", a);
                self.b.build_float_sub(a, fl, "").unwrap()
            }
            Un::Sign => {
                let zero = f.const_float(0.0);
                let pos = self
                    .b
                    .build_float_compare(FloatPredicate::OGT, a, zero, "")
                    .unwrap();
                let neg = self
                    .b
                    .build_float_compare(FloatPredicate::OLT, a, zero, "")
                    .unwrap();
                let r = self
                    .b
                    .build_select(neg, f.const_float(-1.0), zero, "")
                    .unwrap()
                    .into_float_value();
                self.b
                    .build_select(pos, f.const_float(1.0), r, "")
                    .unwrap()
                    .into_float_value()
            }
            Un::Not => self.flag(FloatPredicate::OEQ, a, f.const_float(0.0)),
            Un::Hash => {
                let i64t = self.ctx.i64_type();
                let k = |v: u64| i64t.const_int(v, false);
                let mut z = self.b.build_bit_cast(a, i64t, "").unwrap().into_int_value();
                z = self
                    .b
                    .build_int_add(z, k(0x9E37_79B9_7F4A_7C15), "")
                    .unwrap();
                let s = self.b.build_right_shift(z, k(30), false, "").unwrap();
                z = self.b.build_xor(z, s, "").unwrap();
                z = self
                    .b
                    .build_int_mul(z, k(0xBF58_476D_1CE4_E5B9), "")
                    .unwrap();
                let s = self.b.build_right_shift(z, k(27), false, "").unwrap();
                z = self.b.build_xor(z, s, "").unwrap();
                z = self
                    .b
                    .build_int_mul(z, k(0x94D0_49BB_1331_11EB), "")
                    .unwrap();
                let s = self.b.build_right_shift(z, k(31), false, "").unwrap();
                z = self.b.build_xor(z, s, "").unwrap();
                let top = self.b.build_right_shift(z, k(11), false, "").unwrap();
                let fl = self.b.build_unsigned_int_to_float(top, f, "").unwrap();
                self.b
                    .build_float_div(fl, f.const_float((1u64 << 53) as f64), "")
                    .unwrap()
            }
            op => self.call_addr(
                un_thunk(op).expect("a thunk for every other op"),
                &[f.into()],
                &[a.into()],
            ),
        }
    }

    fn bin(&self, op: Bin, a: FloatValue<'static>, b: FloatValue<'static>) -> FloatValue<'static> {
        let f = self.f64t();
        match op {
            Bin::Add => self.b.build_float_add(a, b, "").unwrap(),
            Bin::Sub => self.b.build_float_sub(a, b, "").unwrap(),
            Bin::Mul => self.b.build_float_mul(a, b, "").unwrap(),
            Bin::Div => self.b.build_float_div(a, b, "").unwrap(),
            Bin::Min | Bin::Max => {
                let nan = self
                    .b
                    .build_float_compare(FloatPredicate::UNO, a, a, "")
                    .unwrap();
                let better = self
                    .b
                    .build_float_compare(
                        if op == Bin::Min {
                            FloatPredicate::OLT
                        } else {
                            FloatPredicate::OGT
                        },
                        b,
                        a,
                        "",
                    )
                    .unwrap();
                let c = self.b.build_or(nan, better, "").unwrap();
                self.b.build_select(c, b, a, "").unwrap().into_float_value()
            }
            Bin::Lt => self.flag(FloatPredicate::OLT, a, b),
            Bin::Le => self.flag(FloatPredicate::OLE, a, b),
            Bin::Gt => self.flag(FloatPredicate::OGT, a, b),
            Bin::Ge => self.flag(FloatPredicate::OGE, a, b),
            Bin::Eq => self.flag(FloatPredicate::OEQ, a, b),
            Bin::Ne => self.flag(FloatPredicate::UNE, a, b),
            Bin::And | Bin::Or => {
                let zero = f.const_float(0.0);
                let x = self
                    .b
                    .build_float_compare(FloatPredicate::UNE, a, zero, "")
                    .unwrap();
                let y = self
                    .b
                    .build_float_compare(FloatPredicate::UNE, b, zero, "")
                    .unwrap();
                let c = if op == Bin::And {
                    self.b.build_and(x, y, "").unwrap()
                } else {
                    self.b.build_or(x, y, "").unwrap()
                };
                self.b
                    .build_select(c, f.const_float(1.0), zero, "")
                    .unwrap()
                    .into_float_value()
            }
            op => self.call_addr(
                bin_thunk(op).expect("a thunk for every other op"),
                &[f.into(), f.into()],
                &[a.into(), b.into()],
            ),
        }
    }

    fn select(
        &self,
        c: FloatValue<'static>,
        a: FloatValue<'static>,
        b: FloatValue<'static>,
    ) -> FloatValue<'static> {
        let cond = self
            .b
            .build_float_compare(FloatPredicate::UNE, c, self.f64t().const_float(0.0), "")
            .unwrap();
        self.b
            .build_select(cond, a, b, "")
            .unwrap()
            .into_float_value()
    }

    fn noise(&self, dims: u8, seed: u32, args: [FloatValue<'static>; 3]) -> FloatValue<'static> {
        let (i32t, f) = (self.ctx.i32_type(), self.f64t());
        self.call_addr(
            noise_thunk_addr(),
            &[i32t.into(), i32t.into(), f.into(), f.into(), f.into()],
            &[
                i32t.const_int(dims as u64, false).into(),
                i32t.const_int(seed as u64, false).into(),
                args[0].into(),
                args[1].into(),
                args[2].into(),
            ],
        )
    }
}

impl Program {
    /// Compile with LLVM (O3). A spike: see `docs/DECISIONS.md`.
    pub fn compile_llvm(&self) -> Result<LlvmCompiled, LlvmError> {
        let err = |what: &str, e: &dyn std::fmt::Display| LlvmError(format!("{what}: {e}"));
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|e| err("initialize", &e))?;
        let ctx: &'static Context = Box::leak(Box::new(Context::create()));
        let module = ctx.create_module("kernel");
        let builder = ctx.create_builder();
        let (f64t, i64t, ptr) = (
            ctx.f64_type(),
            ctx.i64_type(),
            ctx.ptr_type(AddressSpace::default()),
        );
        let fn_ty = ctx.void_type().fn_type(
            &[i64t.into(), ptr.into(), ptr.into(), ptr.into(), ptr.into()],
            false,
        );
        let func = module.add_function("kernel", fn_ty, None);
        let entry = ctx.append_basic_block(func, "entry");
        let header = ctx.append_basic_block(func, "header");
        let body = ctx.append_basic_block(func, "body");
        let exit = ctx.append_basic_block(func, "exit");
        builder.position_at_end(entry);
        let p = |k: u32| func.get_nth_param(k).unwrap();
        let (n, points, frame, params, outs) = (
            p(0).into_int_value(),
            p(1).into_pointer_value(),
            p(2).into_pointer_value(),
            p(3).into_pointer_value(),
            p(4).into_pointer_value(),
        );
        let slots = self.input_slots();
        let varying = self.varying();
        let em = Em {
            ctx,
            b: &builder,
            m: &module,
        };
        let gep = |base: inkwell::values::PointerValue<'static>,
                   idx: IntValue<'static>,
                   ty: inkwell::types::BasicTypeEnum<'static>| unsafe {
            builder.build_in_bounds_gep(ty, base, &[idx], "").unwrap()
        };
        let k = |v: u64| i64t.const_int(v, false);
        let n_point = slots.iter().filter(|s| s.0 == Rate::Point).count();
        let cols: Vec<_> = (0..n_point)
            .map(|c| {
                builder
                    .build_load(ptr, gep(points, k(c as u64), ptr.into()), "")
                    .unwrap()
                    .into_pointer_value()
            })
            .collect();
        let out_ptrs: Vec<_> = (0..self.outputs.len())
            .map(|c| {
                builder
                    .build_load(ptr, gep(outs, k(c as u64), ptr.into()), "")
                    .unwrap()
                    .into_pointer_value()
            })
            .collect();
        let mut vals: Vec<Option<FloatValue<'static>>> = vec![None; self.nodes.len()];
        let get = |vals: &Vec<Option<FloatValue<'static>>>, id: Id| {
            vals[id as usize].expect("operands come first")
        };
        let compute = |node: &Node,
                       vals: &Vec<Option<FloatValue<'static>>>,
                       i: Option<IntValue<'static>>|
         -> FloatValue<'static> {
            match *node {
                Node::Const(bits) => f64t.const_float(f64::from_bits(bits)),
                Node::Param(c) => builder
                    .build_load(f64t, gep(params, k(c as u64), f64t.into()), "")
                    .unwrap()
                    .into_float_value(),
                Node::Input(c) => match slots[c as usize] {
                    (Rate::Frame, s) => builder
                        .build_load(f64t, gep(frame, k(s as u64), f64t.into()), "")
                        .unwrap()
                        .into_float_value(),
                    (Rate::Point, s) => builder
                        .build_load(
                            f64t,
                            gep(cols[s], i.expect("point inputs are varying"), f64t.into()),
                            "",
                        )
                        .unwrap()
                        .into_float_value(),
                },
                Node::Un(op, a) => em.un(op, get(vals, a)),
                Node::Bin(op, a, b) => em.bin(op, get(vals, a), get(vals, b)),
                Node::Select(c, a, b) => em.select(get(vals, c), get(vals, a), get(vals, b)),
                Node::Noise { dims, seed, args } => em.noise(
                    dims,
                    seed,
                    [get(vals, args[0]), get(vals, args[1]), get(vals, args[2])],
                ),
            }
        };
        for (idx, node) in self.nodes.iter().enumerate() {
            if !varying[idx] {
                vals[idx] = Some(compute(node, &vals, None));
            }
        }
        builder.build_unconditional_branch(header).unwrap();
        builder.position_at_end(header);
        let phi = builder.build_phi(i64t, "i").unwrap();
        phi.add_incoming(&[(&k(0), entry)]);
        let i = phi.as_basic_value().into_int_value();
        let more = builder
            .build_int_compare(IntPredicate::ULT, i, n, "")
            .unwrap();
        builder.build_conditional_branch(more, body, exit).unwrap();
        builder.position_at_end(body);
        let mut pv = vals.clone();
        for (idx, node) in self.nodes.iter().enumerate() {
            if varying[idx] {
                pv[idx] = Some(compute(node, &pv, Some(i)));
            }
        }
        for (c, o) in self.outputs.iter().enumerate() {
            builder
                .build_store(
                    gep(out_ptrs[c], i, f64t.into()),
                    pv[*o as usize].expect("computed"),
                )
                .unwrap();
        }
        let next = builder.build_int_add(i, k(1), "").unwrap();
        builder.build_unconditional_branch(header).unwrap();
        phi.add_incoming(&[(&next, body)]);
        builder.position_at_end(exit);
        builder.build_return(None).unwrap();
        module.verify().map_err(|e| err("verify", &e.to_string()))?;

        // the O3 pipeline for the host CPU (vectorization included), then machine code
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| err("target", &e))?;
        let (cpu, feats) = (
            TargetMachine::get_host_cpu_name(),
            TargetMachine::get_host_cpu_features(),
        );
        let tm = target
            .create_target_machine(
                &triple,
                cpu.to_str().unwrap_or("generic"),
                feats.to_str().unwrap_or(""),
                OptimizationLevel::Aggressive,
                RelocMode::Default,
                CodeModel::JITDefault,
            )
            .ok_or_else(|| LlvmError("no target machine".into()))?;
        module.set_triple(&triple);
        module.set_data_layout(&tm.get_target_data().get_data_layout());
        module
            .run_passes("default<O3>", &tm, PassBuilderOptions::create())
            .map_err(|e| err("passes", &e.to_string()))?;
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::Aggressive)
            .map_err(|e| err("engine", &e.to_string()))?;
        // SAFETY: the function has exactly this signature
        let kernel: Kernel = unsafe {
            engine
                .get_function::<Kernel>("kernel")
                .map_err(|e| err("lookup", &e.to_string()))?
                .as_raw()
        };
        Ok(LlvmCompiled {
            prog: self.clone(),
            kernel,
            _engine: engine,
        })
    }
}

impl LlvmCompiled {
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
        // SAFETY: `slots` checked the columns and counts; every output has `n` numbers
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
}
