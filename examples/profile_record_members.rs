//! Measure tokenization, AST construction and teardown for record-heavy inputs.

use dynamite_compiler::{
    ast::{AstBuilder, AstBuilderImpl},
    error::ErrorPrinter,
    tokenizer::Tokenizer,
};
use std::{hint::black_box, time::Instant};

fn frontend(code: &str) {
    let logger = ErrorPrinter::new(code);
    let tokens = Tokenizer::tokenize(code, false).unwrap();
    let mut builder = AstBuilderImpl::new(&logger, &tokens);
    black_box(builder.build(false));
}

fn median_ms(code: &str, iterations: usize) -> f64 {
    frontend(code);
    let mut samples = Vec::with_capacity(9);
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..iterations {
            frontend(black_box(code));
        }
        samples.push(start.elapsed().as_secs_f64() * 1_000.0 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    println!("workload\ttokenize_parse_drop_ms");
    for members in [128, 512, 2048] {
        let fields = (0..members)
            .map(|index| format!("int field{index};"))
            .collect::<String>();
        let last = members - 1;
        let code = format!(
            "struct Wide {{{fields}}};int main(void){{struct Wide value;value.field{last}=7;return value.field{last};}}"
        );
        println!("wide-{members}\t{:.4}", median_ms(&code, 5));
    }
    let fields = (0..512)
        .map(|index| format!("int field{index};"))
        .collect::<String>();
    let values = vec!["1"; 512].join(",");
    let code = format!(
        "struct Wide {{{fields}}};int main(void){{struct Wide value={{{values}}};return value.field511;}}"
    );
    println!("initialized-512\t{:.4}", median_ms(&code, 5));
    for path in std::env::args().skip(1) {
        let code = std::fs::read_to_string(&path).unwrap();
        println!("{path}\t{:.4}", median_ms(&code, 1));
    }
}
