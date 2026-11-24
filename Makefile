PREFIX ?= /usr
DESTDIR ?=
BIN := target/release

all:
	cargo build --release --locked

check:
	cargo test --release --locked

install:
	install -Dm755 $(BIN)/document-viewer $(DESTDIR)$(PREFIX)/bin/document-viewer
	install -Dm755 $(BIN)/document-viewer-thumbnailer $(DESTDIR)$(PREFIX)/bin/document-viewer-thumbnailer
	install -Dm644 data/document-viewer.desktop $(DESTDIR)$(PREFIX)/share/applications/document-viewer.desktop
	install -Dm644 data/document-viewer.thumbnailer $(DESTDIR)$(PREFIX)/share/thumbnailers/document-viewer.thumbnailer
	install -Dm644 data/document-viewer.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/document-viewer.svg
	install -Dm644 data/document-viewer-symbolic.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/document-viewer-symbolic.svg

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/document-viewer $(DESTDIR)$(PREFIX)/bin/document-viewer-thumbnailer \
	      $(DESTDIR)$(PREFIX)/share/applications/document-viewer.desktop \
	      $(DESTDIR)$(PREFIX)/share/thumbnailers/document-viewer.thumbnailer \
	      $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/document-viewer.svg \
	      $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/document-viewer-symbolic.svg

.PHONY: all check install uninstall
