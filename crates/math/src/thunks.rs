//! The functions that the machine-code backends call for every operation that is not an exact IEEE instruction: each
//! one runs the very same Rust function (`un_apply`, `bin_apply`, `noise_apply`) as the interpreter, so the results
//! are identical bit for bit.
use crate::ir::*;

macro_rules! un_thunks {
    ($($name:ident = $op:ident),* $(,)?) => {
        $(extern "C" fn $name(x: f64) -> f64 { un_apply(Un::$op, x) })*
        pub(crate) fn un_thunk(op: Un) -> Option<usize> {
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
        pub(crate) fn bin_thunk(op: Bin) -> Option<usize> {
            match op { $(Bin::$op => Some($name as *const () as usize),)* _ => None }
        }
    };
}
bin_thunks! { t_rem = Rem, t_pow = Pow, t_atan2 = Atan2 }
pub(crate) extern "C" fn t_noise(dims: u32, seed: u32, x: f64, y: f64, z: f64) -> f64 {
    noise_apply(dims as u8, seed, x, y, z)
}

pub(crate) fn noise_thunk_addr() -> usize {
    t_noise as *const () as usize
}
