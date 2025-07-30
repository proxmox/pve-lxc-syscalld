include /usr/share/dpkg/architecture.mk
include /usr/share/dpkg/pkg-info.mk

include defines.mk

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
DBG_DEB=$(PACKAGE)-dbgsym_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb
DSC=$(PACKAGE)_$(DEB_VERSION).dsc

BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION_UPSTREAM)

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

$(COMPILED_BINS): cargo-build

install: $(COMPILED_BINS)
	install -dm755 $(DESTDIR)$(LIBEXECDIR)/pve-lxc-syscalld
	$(foreach i,$(SERVICE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBEXECDIR)/pve-lxc-syscalld/ ;)

$(BUILDDIR): src debian etc Cargo.toml
	rm -rf $(BUILDDIR) $(BUILDDIR).tmp
	mkdir $(BUILDDIR).tmp
	#mkdir $(BUILDDIR).tmp/.cargo
	cp -a -t $(BUILDDIR).tmp $^ Makefile defines.mk
	#cp -a -t $(BUILDDIR).tmp/.cargo .cargo/config
	echo "git clone git://git.proxmox.com/git/pve-lxc-syscalld.git\\ngit checkout $(shell git rev-parse HEAD)" > $(BUILDDIR).tmp/debian/SOURCE
	mv $(BUILDDIR).tmp $(BUILDDIR)

.PHONY: deb
deb: $(DEB)
$(DEB) $(DBG_DEB) &: $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc
	lintian $(DEB)

.PHONY: dsc
dsc:
	$(MAKE) clean
	$(MAKE) $(DSC)
	lintian $(DSC)

$(DSC): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -S -us -uc -d

sbuild: $(DSC)
	sbuild $(DSC)

.PHONY: upload
upload: UPLOAD_DIST ?= $(DEB_DISTRIBUTION)
upload: $(DEB) $(DBG_DEB)
	tar -cf - $(DEB) $(DBG_DEB) | ssh -X repoman@repo.proxmox.com upload --product pve --dist $(UPLOAD_DIST)

.PHONY: dinstall
dinstall:
	$(MAKE) deb
	sudo -k dpkg -i $(DEB)

clean:
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C $(i) clean ;)
	rm -rf ./target
	rm -rf ./$(BUILDDIR)
	rm -f -- *.deb *.dsc *.tar.?z *.buildinfo *.build *.changes
