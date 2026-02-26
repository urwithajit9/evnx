#!/usr/bin/env bash
# Complete test suite runner
# Usage: ./scripts/test.sh [unit|integration|coverage|bench|all]

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

# Check if running in CI
if [ -n "$CI" ]; then
    info "Running in CI mode"
    CI_MODE=true
else
    CI_MODE=false
fi

# Parse arguments
TEST_TYPE="${1:-all}"

# Create reports directory
mkdir -p reports

# ============================================================================
# Unit Tests
# ============================================================================

run_unit_tests() {
    info "Running unit tests..."

    if cargo test --lib --no-fail-fast -- --test-threads=1 2>&1 | tee reports/unit-tests.log; then
        success "Unit tests passed"
        return 0
    else
        error "Unit tests failed"
        return 1
    fi
}

# ============================================================================
# Integration Tests
# ============================================================================

run_integration_tests() {
    info "Running integration tests..."

    # Build first to catch compile errors
    if ! cargo build --tests; then
        error "Failed to build tests"
        return 1
    fi

    if cargo test --test tests --no-fail-fast 2>&1 | tee reports/integration-tests.log; then
        success "Integration tests passed"
        return 0
    else
        error "Integration tests failed"
        return 1
    fi
}

# ============================================================================
# Coverage
# ============================================================================

run_coverage() {
    info "Running coverage analysis..."

    # Check if tarpaulin is installed
    if ! command -v cargo-tarpaulin &> /dev/null; then
        warn "cargo-tarpaulin not installed, installing..."
        cargo install cargo-tarpaulin
    fi

    info "Generating coverage report (this may take a few minutes)..."

    if cargo tarpaulin \
        --out Html \
        --out Xml \
        --output-dir reports/coverage \
        --exclude-files 'target/*' \
        --exclude-files 'tests/*' \
        --timeout 300 2>&1 | tee reports/coverage.log; then

        # Extract coverage percentage
        COVERAGE=$(grep -oP 'Coverage: \K[0-9.]+' reports/coverage.log | tail -1)

        if [ -n "$COVERAGE" ]; then
            info "Coverage: ${COVERAGE}%"

            if (( $(echo "$COVERAGE >= 80" | bc -l) )); then
                success "Coverage is above 80% threshold"
            elif (( $(echo "$COVERAGE >= 60" | bc -l) )); then
                warn "Coverage is ${COVERAGE}%, target is 80%"
            else
                error "Coverage is ${COVERAGE}%, below 60% minimum"
            fi
        fi

        if [ "$CI_MODE" = false ]; then
            info "Opening coverage report..."
            if command -v xdg-open &> /dev/null; then
                xdg-open reports/coverage/index.html
            elif command -v open &> /dev/null; then
                open reports/coverage/index.html
            fi
        fi

        success "Coverage report generated: reports/coverage/index.html"
        return 0
    else
        error "Coverage analysis failed"
        return 1
    fi
}

# ============================================================================
# Benchmarks
# ============================================================================

run_benchmarks() {
    info "Running benchmarks..."

    if cargo bench --no-fail-fast 2>&1 | tee reports/benchmarks.log; then
        success "Benchmarks completed"

        if [ "$CI_MODE" = false ]; then
            info "Benchmark results saved in target/criterion/"
        fi

        return 0
    else
        error "Benchmarks failed"
        return 1
    fi
}

# ============================================================================
# All Tests
# ============================================================================

run_all_tests() {
    info "Running complete test suite..."
    echo

    local failed=0

    # Unit tests
    if ! run_unit_tests; then
        ((failed++))
    fi
    echo

    # Integration tests
    if ! run_integration_tests; then
        ((failed++))
    fi
    echo

    # Coverage (only if tests passed)
    if [ $failed -eq 0 ]; then
        if ! run_coverage; then
            warn "Coverage failed but tests passed"
        fi
    else
        warn "Skipping coverage due to test failures"
    fi
    echo

    # Benchmarks (only if not in CI)
    if [ "$CI_MODE" = false ]; then
        if ! run_benchmarks; then
            warn "Benchmarks failed"
        fi
    else
        info "Skipping benchmarks in CI mode"
    fi
    echo

    # Summary
    echo "========================================"
    echo "Test Summary"
    echo "========================================"

    if [ $failed -eq 0 ]; then
        success "All tests passed! ✓"
        echo
        echo "Reports generated in reports/ directory:"
        ls -lh reports/
        return 0
    else
        error "Some tests failed"
        echo "Failed test suites: $failed"
        return 1
    fi
}

# ============================================================================
# Quick Tests (Pre-commit)
# ============================================================================

run_quick_tests() {
    info "Running quick tests (unit only)..."

    if cargo test --lib --quiet; then
        success "Quick tests passed"
        return 0
    else
        error "Quick tests failed"
        return 1
    fi
}

# ============================================================================
# Linting and Formatting
# ============================================================================

run_lint() {
    info "Running linting checks..."

    local failed=0

    # Format check
    info "Checking formatting..."
    if cargo fmt --check; then
        success "Formatting OK"
    else
        error "Formatting issues found (run: cargo fmt)"
        ((failed++))
    fi

    # Clippy
    info "Running clippy..."
    if cargo clippy --all-features -- -D warnings; then
        success "Clippy OK"
    else
        error "Clippy issues found"
        ((failed++))
    fi

    # Doc check
    info "Checking documentation..."
    if cargo doc --no-deps --quiet; then
        success "Documentation OK"
    else
        error "Documentation issues found"
        ((failed++))
    fi

    return $failed
}

# ============================================================================
# Main
# ============================================================================

main() {
    echo "========================================"
    echo "evnx Test Suite"
    echo "========================================"
    echo

    case "$TEST_TYPE" in
        unit)
            run_unit_tests
            ;;
        integration)
            run_integration_tests
            ;;
        coverage)
            run_coverage
            ;;
        bench|benchmark)
            run_benchmarks
            ;;
        quick)
            run_quick_tests
            ;;
        lint)
            run_lint
            ;;
        all)
            run_all_tests
            ;;
        *)
            error "Unknown test type: $TEST_TYPE"
            echo
            echo "Usage: $0 [unit|integration|coverage|bench|quick|lint|all]"
            echo
            echo "Test types:"
            echo "  unit        - Run unit tests only"
            echo "  integration - Run integration tests only"
            echo "  coverage    - Generate coverage report"
            echo "  bench       - Run benchmarks"
            echo "  quick       - Quick pre-commit tests"
            echo "  lint        - Run linting checks"
            echo "  all         - Run everything (default)"
            exit 1
            ;;
    esac

    exit $?
}

main