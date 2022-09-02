# go-pion-webrtc Makefile

.PHONY: all publish test static docs tools tool_rust tool_fmt tool_readme

SHELL = /usr/bin/env sh

all: test

publish:
	@case "$(crate)" in \
		tx4-core) \
			export MANIFEST="./crates/tx4-core/Cargo.toml"; \
			;; \
		tx4-go-pion-sys) \
			export MANIFEST="./crates/tx4-go-pion-sys/Cargo.toml"; \
			;; \
		tx4-go-pion) \
			export MANIFEST="./crates/tx4-go-pion/Cargo.toml"; \
			;; \
		tx4-signal) \
			export MANIFEST="./crates/tx4-signal/Cargo.toml"; \
			;; \
		tx4-signal-srv) \
			export MANIFEST="./crates/tx4-signal-srv/Cargo.toml"; \
			;; \
		tx4-demo) \
			export MANIFEST="./crates/tx4-demo/Cargo.toml"; \
			;; \
		*) \
			echo "USAGE: make publish crate=tx4-core"; \
			echo "USAGE: make publish crate=tx4-go-pion-sys"; \
			echo "USAGE: make publish crate=tx4-go-pion"; \
			echo "USAGE: make publish crate=tx4-signal"; \
			echo "USAGE: make publish crate=tx4-signal-srv"; \
			echo "USAGE: make publish crate=tx4-demo"; \
			exit 1; \
			;; \
	esac; \
	export VER="v$$(grep version $${MANIFEST} | head -1 | cut -d ' ' -f 3 | cut -d \" -f 2)"; \
	echo "publish $(crate) $${MANIFEST} $${VER}"; \
	git diff --exit-code; \
	cargo publish --manifest-path $${MANIFEST}; \
	git tag -a "$(crate)-$${VER}" -m "$(crate)-$${VER}"; \
	git push --tags;

test: static tools
	RUST_BACKTRACE=1 cargo test

static: docs tools
	(cd crates/tx4-go-pion-sys; go fmt)
	cargo fmt -- --check
	cargo clippy
	@if [ "${CI}x" != "x" ]; then git diff --exit-code; fi

docs: tools
	printf '### The `tx4-signal-srv` executable\n`tx4-signal-srv --help`\n```text\n' > crates/tx4-signal-srv/src/docs/srv_help.md
	cargo run --manifest-path crates/tx4-signal-srv/Cargo.toml -- --help >> crates/tx4-signal-srv/src/docs/srv_help.md
	printf '\n```\n' >> crates/tx4-signal-srv/src/docs/srv_help.md
	cargo readme -r crates/tx4-signal-srv -o README.md
	printf '\n' >> crates/tx4-signal-srv/README.md
	cat crates/tx4-signal-srv/src/docs/srv_help.md >> crates/tx4-signal-srv/README.md
	cargo readme -r crates/tx4-core -o README.md
	cargo readme -r crates/tx4-go-pion-sys -o README.md
	cargo readme -r crates/tx4-go-pion -o README.md
	cargo readme -r crates/tx4-signal -o README.md
	cargo readme -r crates/tx4-demo -o README.md

tools: tool_rust tool_fmt tool_clippy tool_readme

tool_rust:
	@if rustup --version >/dev/null 2>&1; then \
		echo "# Makefile # found rustup, setting override stable"; \
		rustup override set stable; \
	else \
		echo "# Makefile # rustup not found, hopefully we're on stable"; \
	fi;

tool_fmt: tool_rust
	@if ! (cargo fmt --version); \
	then \
		if rustup --version >/dev/null 2>&1; then \
			echo "# Makefile # installing rustfmt with rustup"; \
			rustup component add rustfmt; \
		else \
			echo "# Makefile # rustup not found, cannot install rustfmt"; \
			exit 1; \
		fi; \
	else \
		echo "# Makefile # rustfmt ok"; \
	fi;

tool_clippy: tool_rust
	@if ! (cargo clippy --version); \
	then \
		if rustup --version >/dev/null 2>&1; then \
			echo "# Makefile # installing clippy with rustup"; \
			rustup component add clippy; \
		else \
			echo "# Makefile # rustup not found, cannot install clippy"; \
			exit 1; \
		fi; \
	else \
		echo "# Makefile # clippy ok"; \
	fi;

tool_readme: tool_rust
	@if ! (cargo readme --version); \
	then \
		cargo install cargo-readme; \
	else \
		echo "# Makefile # readme ok"; \
	fi;
