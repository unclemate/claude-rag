//! Performance benchmarks for document parsing.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_parse_markdown(c: &mut Criterion) {
    let simple_md = "# Title\n\nParagraph with **bold** text.";
    let medium_md = "# Main Title\n\n## Section 1\n\nContent with [link](url) and **bold**.\n\n## Section 2\n\nMore content.";
    let large_md = {
        let mut s = String::new();
        for i in 0..50 {
            s.push_str(&format!("## Section {}\n\nContent with **bold** and *italic* and [link](url).\n\n", i));
        }
        s
    };

    c.bench_function("parse_markdown_simple", |b| {
        b.iter(|| {
            let parser = claude_rag::document::DocumentParser::new(claude_rag::document::DocumentFormat::Markdown);
            let path = std::path::Path::new("test.md");
            black_box(parser.parse(black_box(simple_md), path, "test-1").unwrap());
        });
    });

    c.bench_function("parse_markdown_medium", |b| {
        b.iter(|| {
            let parser = claude_rag::document::DocumentParser::new(claude_rag::document::DocumentFormat::Markdown);
            let path = std::path::Path::new("test.md");
            black_box(parser.parse(black_box(medium_md), path, "test-1").unwrap());
        });
    });

    c.bench_function("parse_markdown_large", |b| {
        b.iter(|| {
            let parser = claude_rag::document::DocumentParser::new(claude_rag::document::DocumentFormat::Markdown);
            let path = std::path::Path::new("test.md");
            black_box(parser.parse(black_box(&large_md), path, "test-1").unwrap());
        });
    });
}

criterion_group!(benches, bench_parse_markdown);

criterion_main!(benches);
