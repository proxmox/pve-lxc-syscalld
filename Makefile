include /usr/share/dpkg/architecture.mk
include /usr/share/dpkg/pkg-info.mk

include defines.mk

GITVERSION:=$(shell git rev-parse HEAD)

SUBDIRS := etc

ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
COMPILEDIR := target/release
else
COMPILEDIR := target/debug
endif

SERVICE_BIN := pve-lxc-syscalld

COMPILED_BINS := \
	$(addprefix $(COMPILEDIR)/,$(SERVICE_BIN))

DEB=$(PACKAGE)_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb
DSC=rust-$(PACKAGE)_$(DEB_VERSION).dsc

all: cargo-build $(SUBDIRS)

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	$(MAKE) -C $@

.PHONY: cargo-build
cargo-build:
	cargo build $(CARGO_BUILD_ARGS)

.PHONY: test
test:
	cargo test $(CARGO_BUILD_ARGS)

.PHONY: check
check: test

.PHONY: san
san:
	cargo +nightly fmt -- --check
	cargo clippy
	cargo test

$(COMPILED_BINS): cargo-build

install: $(COMPILED_BINS)
	install -dm755 $(DESTDIR)$(LIBEXECDIR)/pve-lxc-syscalld
	$(foreach i,$(SERVICE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBEXECDIR)/pve-lxc-syscalld/ ;)

.PHONY: build
build:
	rm -rf build
	debcargo package \
	  --config debian/debcargo.toml \
	  --changelog-ready \
	  --no-overlay-write-back \
	  --directory build \
	  pve-lxc-syscalld \
	  $(shell dpkg-parsechangelog -l debian/changelog -SVersion | sed -e 's/-.*//')
	sed -e '1,/^$$/ ! d' build/debian/control > build/debian/control.src
	cat build/debian/control.src build/debian/control.in > build/debian/control
	rm build/debian/control.in build/debian/control.src
	rm build/Cargo.lock
	find build/debian -name "*.hint" -delete
	echo system >build/rust-toolchain
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C build/$(i) clean ;)

.PHONY: deb
deb: $(DEB)
$(DEB): build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean --build-profiles=nodoc
	lintian $(DEB)

upload: deb
	dcmd --deb rust-pve-lxc-syscalld_*.changes \
	    | grep -v '.changes$$' \
	    | tar -cf- -T- \
	    | ssh -X repoman@repo.proxmox.com upload --product pve --dist bullseye

.PHONY: dsc
dsc: $(DSC)
$(DSC): build
	cd build; dpkg-buildpackage -S -us -uc -d -nc
	lintian $(DSC)

.PHONY: dinstall
dinstall:
	$(MAKE) deb
	sudo -k dpkg -i $(DEB)

clean:
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C $(i) clean ;)
	cargo clean
	rm -rf *.deb *.dsc *.tar.gz *.buildinfo *.changes build
