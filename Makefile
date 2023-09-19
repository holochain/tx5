# tx5 Makefile

.PHONY: all publish test static docs tools tool_rust tool_fmt tool_readme

SHELL = /usr/bin/env sh -eu

all: test

publish-all:
	$(MAKE) publish crate=tx5-core
	$(MAKE) publish crate=tx5-online
	$(MAKE) publish crate=tx5-go-pion-turn
	$(MAKE) publish crate=tx5-go-pion-sys
	$(MAKE) publish crate=tx5-go-pion
	$(MAKE) publish crate=tx5-signal-srv
	$(MAKE) publish crate=tx5-signal
	$(MAKE) publish crate=tx5
	$(MAKE) publish crate=tx5-demo

publish:
	@case "$(crate)" in \
		tx5-core) \
			export MANIFEST="./crates/tx5-core/Cargo.toml"; \
			;; \
		tx5-online) \
			export MANIFEST="./crates/tx5-online/Cargo.toml"; \
			;; \
		tx5-go-pion-turn) \
			export MANIFEST="./crates/tx5-go-pion-turn/Cargo.toml"; \
			;; \
		tx5-go-pion-sys) \
			export MANIFEST="./crates/tx5-go-pion-sys/Cargo.toml"; \
			;; \
		tx5-go-pion) \
			export MANIFEST="./crates/tx5-go-pion/Cargo.toml"; \
			;; \
		tx5-signal-srv) \
			export MANIFEST="./crates/tx5-signal-srv/Cargo.toml"; \
			;; \
		tx5-signal) \
			export MANIFEST="./crates/tx5-signal/Cargo.toml"; \
			;; \
		tx5) \
			export MANIFEST="./crates/tx5/Cargo.toml"; \
			;; \
		tx5-demo) \
			export MANIFEST="./crates/tx5-demo/Cargo.toml"; \
			;; \
		*) \
			echo "USAGE: make publish crate=tx5-core"; \
			echo "USAGE: make publish crate=tx5-online"; \
			echo "USAGE: make publish crate=tx5-go-pion-turn"; \
			echo "USAGE: make publish crate=tx5-go-pion-sys"; \
			echo "USAGE: make publish crate=tx5-go-pion"; \
			echo "USAGE: make publish crate=tx5-signal-srv"; \
			echo "USAGE: make publish crate=tx5-signal"; \
			echo "USAGE: make publish crate=tx5"; \
			echo "USAGE: make publish crate=tx5-demo"; \
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
	cargo build --all-targets
	RUST_BACKTRACE=1 cargo test -- --test-threads 1 --nocapture

static: docs tools
	cargo fmt -- --check
	cargo clippy
	(cd crates/tx5-go-pion-sys; go fmt)
	(cd crates/tx5-go-pion-turn; go fmt)
	@if [ "${CI}x" != "x" ]; then git diff --exit-code; fi

docs: tools
	cp -f crates/tx5-core/src/README.tpl README.md
	printf '### The `tx5-signal-srv` executable\n`tx5-signal-srv --help`\n```text\n' > crates/tx5-signal-srv/src/docs/srv_help.md
	cargo run --manifest-path crates/tx5-signal-srv/Cargo.toml -- --help >> crates/tx5-signal-srv/src/docs/srv_help.md
	printf '\n```\n' >> crates/tx5-signal-srv/src/docs/srv_help.md
	cp -f crates/tx5-core/src/README.tpl crates/tx5-signal-srv/README.md
	cargo rdme --force -w tx5-signal-srv
	printf '\n' >> crates/tx5-signal-srv/README.md
	cat crates/tx5-signal-srv/src/docs/srv_help.md >> crates/tx5-signal-srv/README.md
	cp -f crates/tx5-core/src/README.tpl crates/tx5-core/README.md
	cargo rdme --force -w tx5-core
	cp -f crates/tx5-core/src/README.tpl crates/tx5-online/README.md
	cargo rdme --force -w tx5-online
	cp -f crates/tx5-core/src/README.tpl crates/tx5-go-pion-turn/README.md
	cargo rdme --force -w tx5-go-pion-turn
	cp -f crates/tx5-core/src/README.tpl crates/tx5-go-pion-sys/README.md
	cargo rdme --force -w tx5-go-pion-sys
	cp -f crates/tx5-core/src/README.tpl crates/tx5-go-pion/README.md
	cargo rdme --force -w tx5-go-pion
	cp -f crates/tx5-core/src/README.tpl crates/tx5-signal/README.md
	cargo rdme --force -w tx5-signal
	cp -f crates/tx5-core/src/README.tpl crates/tx5/README.md
	cargo rdme --force -w tx5
	cp -f crates/tx5-core/src/README.tpl crates/tx5-demo/README.md
	cargo rdme --force -w tx5-demo

tools: tool_rust tool_fmt tool_clippy tool_readme

tool_rust:
	@if ! rustup --version >/dev/null 2>&1; then \
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
	@if ! (cargo rdme --version); \
	then \
		cargo install cargo-rdme --version 1.4.0 --locked; \
	else \
		echo "# Makefile # readme ok"; \
	fi;
