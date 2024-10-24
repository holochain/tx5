# tx5 Makefile

.PHONY: all publish-all publish bump test unit static lint dep fmt docs

SHELL = /usr/bin/env sh -eu

all: test

publish-all:
	$(MAKE) publish crate=tx5-core
	$(MAKE) publish crate=tx5-online
	$(MAKE) publish crate=tx5-go-pion-turn
	$(MAKE) publish crate=tx5-go-pion-sys
	$(MAKE) publish crate=tx5-go-pion
	$(MAKE) publish crate=tx5-signal
	$(MAKE) publish crate=tx5-connection
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
		tx5-signal) \
			export MANIFEST="./crates/tx5-signal/Cargo.toml"; \
			;; \
		tx5-connection) \
			export MANIFEST="./crates/tx5-connection/Cargo.toml"; \
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
			echo "USAGE: make publish crate=tx5-signal"; \
			echo "USAGE: make publish crate=tx5-connection"; \
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

bump:
	@if [ "$(ver)x" = "x" ]; then \
		echo "USAGE: make bump ver=0.0.2-alpha"; \
		exit 1; \
	fi
	sed -i 's/^\(tx5[^=]*= { \|\)version = "[^"]*"/\1version = "$(ver)"/g' $$(find . -name Cargo.toml)

test: static unit

unit:
	cargo build --all-targets
	RUST_BACKTRACE=1 RUST_LOG=error cargo test -- --nocapture
	RUST_BACKTRACE=1 RUST_LOG=error cargo test --no-default-features --features backend-go-pion --manifest-path crates/tx5-connection/Cargo.toml -- --nocapture
	RUST_BACKTRACE=1 RUST_LOG=error cargo test --no-default-features --features backend-go-pion --manifest-path crates/tx5/Cargo.toml -- --nocapture

static: dep fmt lint docs
	@if [ "${CI}x" != "x" ]; then git diff --exit-code; fi

lint:
	cargo clippy --features backend-go-pion,backend-libdatachannel -- -Dwarnings

dep:
	@#uhhh... better way to do this? depend on cargo-tree?
	@if [ $$(grep 'name = "sodoken"' Cargo.lock | wc -l) != "1" ]; then echo "ERROR: multiple sodokens"; exit 127; fi

fmt:
	cargo fmt -- --check
	(cd crates/tx5-go-pion-sys; go fmt)
	(cd crates/tx5-go-pion-turn; go fmt)

docs:
	cp -f crates/tx5-core/src/README.tpl README.md
	cp -f crates/tx5-core/src/README.tpl crates/tx5-core/README.md
	cargo rdme --force -w tx5-core
	cp -f crates/tx5-core/src/README.tpl crates/tx5-connection/README.md
	cargo rdme --force -w tx5-connection
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
