//! Frontend rejection cases run once because they fail before backend selection.
//! Assembly without companion C uses one optimization setting: Clang does not
//! optimize assembly input. LLVM IR and companion C retain both optimization levels.

mod cases;
mod support;
