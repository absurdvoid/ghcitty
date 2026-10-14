SHELL := /bin/zsh
ORIG_PATH := $(PATH)
export PATH := $(HOME)/.cargo/bin:$(HOME)/.ghcup/bin:$(HOME)/.local/bin:$(PATH)

.PHONY: build run test fmt clean install

build:
	cargo build --release

fmt:
	cargo fmt

run: build
	./target/release/ghcitty

test:
	cargo test

install:
	cargo install --path .
	codesign --force --sign - $(HOME)/.cargo/bin/ghcitty
	@active="$$(PATH='$(ORIG_PATH)' command -v ghcitty 2>/dev/null)"; \
	expected="$(HOME)/.cargo/bin/ghcitty"; \
	if [ -n "$$active" ] && [ "$$active" != "$$expected" ]; then \
		echo ""; \
		echo "warning: another 'ghcitty' is shadowing the one we just installed:"; \
		echo "    on PATH:        $$active"; \
		echo "    just installed: $$expected"; \
		echo "remove the stale binary (rm $$active) or put $(HOME)/.cargo/bin earlier in PATH."; \
	fi

clean:
	cargo clean
