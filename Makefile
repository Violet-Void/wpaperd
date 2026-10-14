# Build, install and diagnose wpaperd. Run `make help` for the targets.
#
# Generic install:   make && sudo make install
# User-local:        make && make install PREFIX=$HOME/.local
# Arch Linux:        make arch-pkg   (replaces the repo `wpaperd` package through pacman)

PREFIX ?= /usr/local
DESTDIR ?=
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
MANDIR ?= $(DATADIR)/man
DOCDIR ?= $(DATADIR)/doc/wpaperd
ifeq ($(PREFIX),$(HOME)/.local)
SYSTEMD_USER_UNITDIR ?= $(DATADIR)/systemd/user
else
SYSTEMD_USER_UNITDIR ?= $(PREFIX)/lib/systemd/user
endif

CARGO ?= cargo
CARGO_TARGET_DIR ?= target
export CARGO_TARGET_DIR
# Extra cargo features, for example FEATURES=avif (needs libdav1d).
FEATURES ?=
OUT := $(CARGO_TARGET_DIR)/release
ARCH_BUILDDIR := $(abspath $(CARGO_TARGET_DIR))/arch
MAKEPKG_FLAGS ?= -si

BINS := wpaperd wpaperctl

.PHONY: all help build install uninstall arch-pkg doctor

all: build

help:
	@echo 'Targets:'
	@echo '  build       cargo build --release, plus man pages (needs scdoc for wpaperd-output.5)'
	@echo '  install     install the build into $$(DESTDIR)$$(PREFIX) (default PREFIX=/usr/local)'
	@echo '  uninstall   remove the files that install created under the same PREFIX'
	@echo '  arch-pkg    build this commit as wpaperd-violet-git and install it with pacman,'
	@echo '              replacing the Arch repo wpaperd package'
	@echo '  doctor      show which wpaperd/wpaperctl binaries, daemon and service are in use'
	@echo 'Variables: PREFIX DESTDIR BINDIR FEATURES CARGO_TARGET_DIR MAKEPKG_FLAGS'

build:
	$(CARGO) build --release --locked $(if $(FEATURES),--features '$(FEATURES)')
	@if command -v scdoc >/dev/null 2>&1; then \
		scdoc < man/wpaperd-output.5.scd > $(OUT)/man/wpaperd-output.5; \
	else \
		echo 'warning: scdoc not found; wpaperd-output.5 will not be installed' >&2; \
	fi

# `install` deliberately does not depend on `build`, so `sudo make install`
# never runs cargo as root or leaves root-owned files in the target directory.
install:
	@for bin in $(BINS); do \
		test -x '$(OUT)'/$$bin || { echo "error: $(OUT)/$$bin is missing; run 'make' as your user first" >&2; exit 1; }; \
	done
	@if [ -z '$(DESTDIR)' ] && command -v pacman >/dev/null 2>&1; then \
		for bin in $(BINS); do \
			if owner=$$(pacman -Qqo '$(BINDIR)'/$$bin 2>/dev/null); then \
				echo "error: $(BINDIR)/$$bin belongs to the pacman package '$$owner'." >&2; \
				echo "Use 'make arch-pkg' to replace it, or pick another PREFIX (e.g. PREFIX=\$$HOME/.local)." >&2; \
				exit 1; \
			fi; \
		done; \
	fi
	install -Dm755 '$(OUT)/wpaperd' '$(DESTDIR)$(BINDIR)/wpaperd'
	install -Dm755 '$(OUT)/wpaperctl' '$(DESTDIR)$(BINDIR)/wpaperctl'
	install -Dm644 '$(OUT)/man/wpaperd.1' '$(DESTDIR)$(MANDIR)/man1/wpaperd.1'
	install -Dm644 '$(OUT)/man/wpaperctl.1' '$(DESTDIR)$(MANDIR)/man1/wpaperctl.1'
	@if [ -f '$(OUT)/man/wpaperd-output.5' ]; then \
		install -Dm644 '$(OUT)/man/wpaperd-output.5' '$(DESTDIR)$(MANDIR)/man5/wpaperd-output.5'; \
	fi
	@for bin in $(BINS); do \
		install -Dm644 '$(OUT)'/completions/$$bin.bash '$(DESTDIR)$(DATADIR)'/bash-completion/completions/$$bin && \
		install -Dm644 '$(OUT)'/completions/_$$bin '$(DESTDIR)$(DATADIR)'/zsh/site-functions/_$$bin && \
		install -Dm644 '$(OUT)'/completions/$$bin.fish '$(DESTDIR)$(DATADIR)'/fish/vendor_completions.d/$$bin.fish \
		|| exit 1; \
	done
	install -Dm644 README.md '$(DESTDIR)$(DOCDIR)/README.md'
	install -Dm644 docs/motion-scenes.md '$(DESTDIR)$(DOCDIR)/motion-scenes.md'
	install -d '$(DESTDIR)$(SYSTEMD_USER_UNITDIR)'
	sed 's|^ExecStart=.*|ExecStart=$(BINDIR)/wpaperd|' contrib/wpaperd.service \
		> '$(DESTDIR)$(SYSTEMD_USER_UNITDIR)/wpaperd.service'
	chmod 644 '$(DESTDIR)$(SYSTEMD_USER_UNITDIR)/wpaperd.service'

uninstall:
	rm -f '$(DESTDIR)$(BINDIR)/wpaperd' '$(DESTDIR)$(BINDIR)/wpaperctl'
	rm -f '$(DESTDIR)$(MANDIR)/man1/wpaperd.1' '$(DESTDIR)$(MANDIR)/man1/wpaperctl.1' \
		'$(DESTDIR)$(MANDIR)/man5/wpaperd-output.5'
	@for bin in $(BINS); do \
		rm -f '$(DESTDIR)$(DATADIR)'/bash-completion/completions/$$bin \
			'$(DESTDIR)$(DATADIR)'/zsh/site-functions/_$$bin \
			'$(DESTDIR)$(DATADIR)'/fish/vendor_completions.d/$$bin.fish; \
	done
	rm -f '$(DESTDIR)$(DOCDIR)/README.md' '$(DESTDIR)$(DOCDIR)/motion-scenes.md'
	-rmdir '$(DESTDIR)$(DOCDIR)' 2>/dev/null
	rm -f '$(DESTDIR)$(SYSTEMD_USER_UNITDIR)/wpaperd.service'

# makepkg rewrites pkgver inside the PKGBUILD it runs, so it runs on a copy in the
# target directory. It packages the committed HEAD; uncommitted edits are not included.
arch-pkg:
	@command -v makepkg >/dev/null 2>&1 || { echo 'error: makepkg not found (Arch Linux only)' >&2; exit 1; }
	@git diff --quiet HEAD -- || echo "warning: uncommitted changes are not packaged; building $$(git rev-parse --short HEAD)" >&2
	install -Dm644 contrib/arch/PKGBUILD '$(ARCH_BUILDDIR)/PKGBUILD'
	cd '$(ARCH_BUILDDIR)' && \
		WPAPERD_SOURCE="git+file://$(CURDIR)#commit=$$(git -C '$(CURDIR)' rev-parse HEAD)" \
		makepkg $(MAKEPKG_FLAGS)

doctor:
	@WPAPERD_BUILD_DIR='$(OUT)' contrib/wpaperd-doctor
