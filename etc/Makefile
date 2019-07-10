include ../defines.mk

UNITS := pve-lxc-syscalld.service

all: $(UNITS)

clean:
	rm -f $(UNITS)

.SUFFIXES: .service.in .service
.service.in.service:
	sed \
	    -e 's!%LIBEXECDIR%!$(LIBEXECDIR)!g' \
	    $< >$@.tmp
	mv $@.tmp $@
