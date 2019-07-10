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

DEB=$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_HOST_ARCH).deb
DSC=$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION).dsc
BUILDSRC := $(PACKAGE)-$(DEB_VERSION_UPSTREAM)

all: cargo-build $(SUBDIRS)

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	$(MAKE) -C $@

.PHONY: cargo-build
cargo-build:
	cargo build $(CARGO_BUILD_ARGS)

$(COMPILED_BINS): cargo-build

install: $(COMPILED_BINS)
	install -dm755 $(DESTDIR)$(LIBEXECDIR)/proxmox-backup
	$(foreach i,$(SERVICE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBEXECDIR)/proxmox-backup/ ;)

# always re-create this dir
# but also copy the local target/ dir as a build-cache
.PHONY: $(BUILDSRC)
$(BUILDSRC):
	rm -rf $(BUILDSRC)
	cargo build --release
	rsync -a debian Makefile defines.mk Cargo.toml Cargo.lock \
	    src $(SUBDIRS) \
	    target \
	    $(BUILDSRC)/
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C $(BUILDSRC)/$(i) clean ;)

.PHONY: deb
deb: $(DEB)
$(DEB): $(BUILDSRC)
	cd $(BUILDSRC); dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DEB)

.PHONY: dsc
dsc: $(DSC)
$(DSC): $(BUILDSRC)
	cd $(BUILDSRC); dpkg-buildpackage -S -us -uc -d -nc
	lintian $(DSC)

clean:
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C $(i) clean ;)
	cargo clean
	rm -rf *.deb *.dsc *.tar.gz *.buildinfo *.changes $(BUILDSRC)
