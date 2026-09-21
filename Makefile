# Romlens developer entry points. Scripts call rustup's cargo explicitly
# because it owns the cross-compilation targets; day-to-day `cargo` from
# either install (Homebrew or rustup) works for test/build.
CARGO ?= $(HOME)/.cargo/bin/cargo
ifeq ($(wildcard $(CARGO)),)
CARGO := cargo
endif
# cargo runs `rustc` from PATH; rustup's must win over Homebrew's.
export PATH := $(HOME)/.cargo/bin:$(PATH)
XCODE_PROJECT := shells/macos/Romlens.xcodeproj
XCODE_DD := shells/macos/build/DerivedData
XCODEBUILD := xcodebuild -project $(XCODE_PROJECT) -scheme Romlens -destination 'platform=macOS' -derivedDataPath $(XCODE_DD)

.PHONY: help test test-rom swift app app-test ci-local cross docker-test clean

help:
	@echo "make test        cargo fmt --check, clippy -D warnings, cargo test --workspace"
	@echo "make test-rom    the same plus the tests pinned to roms/SuperMetroid.F8DF.sfc"
	@echo "make swift       build the XCFramework + RomlensKit package and run swift test"
	@echo "make app         xcodegen generate + xcodebuild build (needs make swift first)"
	@echo "make app-test    xcodebuild test for the macOS shell"
	@echo "make cross       cargo check for Linux and Windows targets (rustup targets)"
	@echo "make docker-test cargo test --workspace inside rust:1.95 (a real Linux run)"
	@echo "make ci-local    everything above, the local stand-in for CI"

test:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace

test-rom:
	ROMLENS_ROM_DIR=$(CURDIR)/roms $(MAKE) test

swift:
	CARGO=$(CARGO) scripts/build-xcframework.sh
	cd bindings/swift/RomlensKit && swift test

app:
	cd shells/macos && xcodegen generate -q
	$(XCODEBUILD) build -quiet

# If Xcode keeps a stale XCFramework after `make swift`, run:
#   xcodebuild -resolvePackageDependencies -project $(XCODE_PROJECT) -scheme Romlens
# or delete $(XCODE_DD).
app-test: app
	$(XCODEBUILD) test 2>&1 | grep -E '✔|✘|error:|TEST' | grep -v 'DerivedData/.*\.swift'

cross:
	CARGO=$(CARGO) scripts/check-cross.sh

docker-test:
	docker run --rm -v "$(CURDIR)":/w -w /w -e CARGO_TARGET_DIR=/w/target/docker rust:1.95 cargo test --workspace

ci-local: test cross docker-test swift app-test

clean:
	$(CARGO) clean
	rm -rf $(XCODE_DD) bindings/swift/RomlensKit/.build bindings/swift/RomlensKit/.gen
