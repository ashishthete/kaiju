# AgentNexus developer tasks.
# Run `make help` to list available targets.

.DEFAULT_GOAL := help
.PHONY: help build test fmt fmt-check lint check daemon cli install smoke clean

NEXUS_URL ?= http://127.0.0.1:7800

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

build: ## Compile the whole workspace
	cargo build

test: ## Run all unit + integration tests
	cargo test

fmt: ## Format the code
	cargo fmt

fmt-check: ## Verify formatting without changing files
	cargo fmt --check

lint: ## Run clippy, treating warnings as errors
	cargo clippy --all-targets -- -D warnings

check: fmt-check lint test ## Run the full pre-commit gate

daemon: ## Run the daemon (NEXUS_PORT overrides the port)
	cargo run -p nexus-daemon

cli: ## Run the CLI; pass args via ARGS, e.g. make cli ARGS="list"
	cargo run -p nexus-cli -- $(ARGS)

install: ## Install the agentnexus CLI onto your PATH
	cargo install --path crates/nexus-cli

smoke: ## Hit the running daemon's API (no tmux required)
	@curl -fsS $(NEXUS_URL)/health && echo
	@curl -fsS $(NEXUS_URL)/agents && echo

clean: ## Remove build artifacts
	cargo clean
