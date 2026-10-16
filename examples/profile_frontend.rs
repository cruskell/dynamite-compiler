//! Profile the public frontend on already-preprocessed C without subprocess startup.

use dynamite_compiler::{
    ast::{AstBuilder, AstBuilderImpl},
    error::ErrorPrinter,
    generator::llvm::generator::IrGenerator,
    timing::Timer,
    tokenizer::Tokenizer,
};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        4,
        "usage: profile_frontend INPUT ROUNDS OUTPUT.ll"
    );
    let code = std::fs::read_to_string(&args[1]).unwrap();
    let rounds: usize = args[2].parse().unwrap();
    assert!(rounds > 0);
    for round in 0..rounds {
        let total = Timer::new("frontend_total");
        let logger = ErrorPrinter::new(&code);
        let timer = Timer::new("tokenize");
        let tokens = Tokenizer::tokenize(&code, false).unwrap();
        drop(timer);
        let timer = Timer::new("parse");
        let mut builder = AstBuilderImpl::new(&logger, &tokens);
        let ast = builder.build(false);
        drop(timer);
        let timer = Timer::new("generate");
        let output = IrGenerator::new(&logger).generate(ast);
        drop(timer);
        let timer = Timer::new("drop_ast");
        drop(builder);
        drop(timer);
        let timer = Timer::new("drop_tokens");
        drop(tokens);
        drop(timer);
        drop(total);
        if round + 1 == rounds {
            std::fs::write(&args[3], std::hint::black_box(&output)).unwrap();
        }
        std::hint::black_box(output);
    }
}
