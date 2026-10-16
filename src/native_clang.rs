//! Optional Clang 15 integration for preprocessing and LLVM IR lowering.
use std::ffi::{CString, c_char, c_int};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Output};
use std::ptr::null_mut;

unsafe extern "C" {
    fn dynamite_pp(
        argc: c_int,
        argv: *const *const c_char,
        output: *mut *mut c_char,
        output_size: *mut usize,
        diagnostics: *mut *mut c_char,
        diagnostics_size: *mut usize,
    ) -> c_int;
    fn dynamite_lower(
        argc: c_int,
        argv: *const *const c_char,
        ir: *const u8,
        ir_len: usize,
        output: *mut *mut c_char,
        output_size: *mut usize,
        diagnostics: *mut *mut c_char,
        diagnostics_size: *mut usize,
    ) -> c_int;
    fn dynamite_pp_free(data: *mut c_char);
}

/// Run a file preprocessing command without a child process.
pub(crate) fn preprocess(command: &Command) -> std::io::Result<Output> {
    invoke(command, None)
}

/// Lower LLVM IR using the same Clang backend options as the subprocess driver.
pub fn lower(command: &Command, ir: &str) -> std::io::Result<Output> {
    invoke(command, Some(ir))
}

fn invoke(command: &Command, ir: Option<&str>) -> std::io::Result<Output> {
    let arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|arg| CString::new(arg.as_encoded_bytes()).map_err(std::io::Error::other))
        .collect::<std::io::Result<Vec<_>>>()?;
    let pointers = arguments.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();
    let argc = c_int::try_from(pointers.len()).map_err(std::io::Error::other)?;
    let mut data = null_mut();
    let mut len = 0;
    let mut diagnostics = null_mut();
    let mut diagnostics_len = 0;
    // The bridge copies output into malloc-owned buffers, including on errors.
    // Copy before releasing them so library callers can use any Rust allocator.
    unsafe {
        let status = if let Some(ir) = ir {
            dynamite_lower(
                argc,
                pointers.as_ptr(),
                ir.as_ptr(),
                ir.len(),
                &mut data,
                &mut len,
                &mut diagnostics,
                &mut diagnostics_len,
            )
        } else {
            dynamite_pp(
                argc,
                pointers.as_ptr(),
                &mut data,
                &mut len,
                &mut diagnostics,
                &mut diagnostics_len,
            )
        };
        let stdout = std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec();
        let stderr = std::slice::from_raw_parts(diagnostics.cast::<u8>(), diagnostics_len).to_vec();
        dynamite_pp_free(data);
        dynamite_pp_free(diagnostics);
        Ok(Output {
            status: ExitStatus::from_raw(status << 8),
            stdout,
            stderr,
        })
    }
}
