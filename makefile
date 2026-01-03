PREFIX ?= /usr/local
name := leguinvim
bin := target/release/$(name)
src := $(shell find -name '*.rs')
install_path := $(DEST_DIR)$(PREFIX)/bin/$(name)

all: $(bin)
	# ./$(bin) u-k* --no-fork >LOG 2> LOG || less LOG
	./$(bin) u-k*

$(bin): $(src) Cargo.toml
	cargo build --release

install: $(bin)
	cp $< $(install_path)

uninstall:
	rm -f $(install_path)

clean:
	cargo clean

test:
	RUST_BACKTRACE=1 cargo test

.PHONY: all install uninstall clean test
