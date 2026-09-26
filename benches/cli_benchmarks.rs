//! Benchmarks for the evnx CLI's hot paths: parsing, secret detection, entropy,
//! and conversion.
//!
//! Run with `cargo bench`.
//!
//! ⚠️ These had never compiled. The file referenced `dotenv_space` — the crate's
//! name before it was renamed to `evnx` — and `criterion` was not a dependency at
//! all, so `cargo bench` failed on a missing crate before it reached the rename.
//! CI never noticed because its clippy step runs `--all-features` without
//! `--all-targets`, and `cargo test` does not build benches.
//!
//! So every number these would have produced is hypothetical: there is no
//! historical baseline to compare against, and the first run establishes one.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use evnx::core::{Parser, ParserConfig};
// ⚠️ `IndexMap`, not `HashMap`. The converter moved to `IndexMap` when `init`'s
// non-determinism was fixed — a `HashMap`'s iteration order is randomised per
// process, so JSON output changed between runs. The benches still said `HashMap`
// because nothing compiled them.
use indexmap::IndexMap;

// Sample .env content for benchmarking
const SMALL_ENV: &str = r#"
DATABASE_URL=postgresql://localhost:5432/db
SECRET_KEY=abc123
DEBUG=True
"#;

const MEDIUM_ENV: &str = r#"
# Database
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb
DB_POOL_SIZE=10
DB_TIMEOUT=30

# Cache
REDIS_URL=redis://localhost:6379/0
REDIS_POOL_SIZE=5

# Application
SECRET_KEY=my-secret-key-here
DEBUG=True
LOG_LEVEL=info
APP_NAME=MyApp
APP_VERSION=1.0.0

# AWS
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
AWS_REGION=us-east-1
AWS_BUCKET=my-bucket

# Third-party
STRIPE_SECRET_KEY=sk_test_123
STRIPE_PUBLISHABLE_KEY=pk_test_456
SENTRY_DSN=https://abc@sentry.io/123
"#;

fn generate_large_env(size: usize) -> String {
    let mut content = String::new();
    for i in 0..size {
        content.push_str(&format!("VAR_{:04}=value_{:04}\n", i, i));
    }
    content
}

// ============================================================================
// PARSER BENCHMARKS
// ============================================================================

fn bench_parser_small(c: &mut Criterion) {
    c.bench_function("parser_small", |b| {
        let parser = Parser::default();
        b.iter(|| {
            parser.parse_content(black_box(SMALL_ENV)).unwrap();
        });
    });
}

fn bench_parser_medium(c: &mut Criterion) {
    c.bench_function("parser_medium", |b| {
        let parser = Parser::default();
        b.iter(|| {
            parser.parse_content(black_box(MEDIUM_ENV)).unwrap();
        });
    });
}

fn bench_parser_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_large");

    for size in [100, 500, 1000].iter() {
        let env_content = generate_large_env(*size);
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            let parser = Parser::default();
            b.iter(|| {
                parser.parse_content(black_box(&env_content)).unwrap();
            });
        });
    }

    group.finish();
}

fn bench_parser_with_expansion(c: &mut Criterion) {
    let content = r#"
BASE=http://localhost
API_URL=${BASE}/api
FULL_URL=${API_URL}/v1
NESTED=${FULL_URL}/nested
"#;

    c.bench_function("parser_with_expansion", |b| {
        let parser = Parser::default();
        b.iter(|| {
            parser.parse_content(black_box(content)).unwrap();
        });
    });
}

fn bench_parser_no_expansion(c: &mut Criterion) {
    let content = r#"
BASE=http://localhost
API_URL=${BASE}/api
FULL_URL=${API_URL}/v1
NESTED=${FULL_URL}/nested
"#;

    c.bench_function("parser_no_expansion", |b| {
        let parser = Parser::new(ParserConfig {
            allow_expansion: false,
            ..Default::default()
        });
        b.iter(|| {
            parser.parse_content(black_box(content)).unwrap();
        });
    });
}

// ============================================================================
// SECRET SCANNING BENCHMARKS
// ============================================================================

fn bench_secret_detection(c: &mut Criterion) {
    // ⚠️ Measures the rule engine, not the removed `detect_secret` chain. The
    // provider patterns are `builtin_rules()` now and run through one RegexSet
    // pass, which is the thing worth timing.
    use evnx::commands::scan::patternset::PatternSet;
    use evnx::utils::patterns::builtin_rules;
    let set = PatternSet::compile(&builtin_rules()).expect("built-ins compile");

    let test_cases = vec![
        ("AWS_KEY", "AKIA4OZRMFJ3VREALKEY"),
        ("STRIPE", "sk_live_51Habc123def456ghi789jkl"),
        ("GITHUB", "ghp_abc123def456ghi789jkl012mno345pqr678"),
        ("NORMAL", "just-a-normal-value"),
    ];

    c.bench_function("secret_detection", |b| {
        b.iter(|| {
            for (key, value) in &test_cases {
                let _ = set.strongest(black_box(value));
                let _ = black_box(key);
            }
        });
    });
}

fn bench_entropy_calculation(c: &mut Criterion) {
    use evnx::utils::patterns::calculate_entropy;

    let test_strings = vec!["aaaaaaa", "abcdefg", "a1b2c3d4e5f6g7h8", "aB3$xY9!zQ2#mK7"];

    c.bench_function("entropy_calculation", |b| {
        b.iter(|| {
            for s in &test_strings {
                calculate_entropy(black_box(s));
            }
        });
    });
}

// ============================================================================
// CONVERTER BENCHMARKS
// ============================================================================

fn bench_convert_to_json(c: &mut Criterion) {
    use evnx::core::converter::{ConvertOptions, Converter};
    use evnx::formats::json::JsonConverter;

    let mut vars = IndexMap::new();
    for i in 0..50 {
        vars.insert(format!("KEY_{}", i), format!("value_{}", i));
    }

    c.bench_function("convert_to_json", |b| {
        let converter = JsonConverter;
        let options = ConvertOptions::default();
        b.iter(|| {
            converter
                .convert(black_box(&vars), black_box(&options))
                .unwrap();
        });
    });
}

fn bench_convert_with_filtering(c: &mut Criterion) {
    use evnx::core::converter::{ConvertOptions, Converter};
    use evnx::formats::json::JsonConverter;

    let mut vars = IndexMap::new();
    for i in 0..50 {
        vars.insert(format!("AWS_{}", i), format!("value_{}", i));
        vars.insert(format!("DB_{}", i), format!("value_{}", i));
    }

    c.bench_function("convert_with_filtering", |b| {
        let converter = JsonConverter;
        let options = ConvertOptions {
            include_pattern: Some("AWS_*".to_string()),
            ..Default::default()
        };
        b.iter(|| {
            converter
                .convert(black_box(&vars), black_box(&options))
                .unwrap();
        });
    });
}

// ============================================================================
// VALIDATION BENCHMARKS
// ============================================================================

fn bench_placeholder_detection(c: &mut Criterion) {
    let test_values = vec![
        "YOUR_KEY_HERE",
        "sk_test_CHANGE_ME",
        "generate-with-openssl",
        "sk_live_51Hrealkey",
        "postgresql://localhost:5432/db",
    ];

    c.bench_function("placeholder_detection", |b| {
        b.iter(|| {
            for value in &test_values {
                // Simulate placeholder check logic
                let lower = value.to_lowercase();
                let _is_placeholder = lower.contains("your_key_here")
                    || lower.contains("change_me")
                    || lower.contains("generate-with");
            }
        });
    });
}

criterion_group!(
    parser_benches,
    bench_parser_small,
    bench_parser_medium,
    bench_parser_large,
    bench_parser_with_expansion,
    bench_parser_no_expansion
);

criterion_group!(
    scanner_benches,
    bench_secret_detection,
    bench_entropy_calculation
);

criterion_group!(
    converter_benches,
    bench_convert_to_json,
    bench_convert_with_filtering
);

criterion_group!(validation_benches, bench_placeholder_detection);

criterion_main!(
    parser_benches,
    scanner_benches,
    converter_benches,
    validation_benches
);
