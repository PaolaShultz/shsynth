PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

.PHONY: build test install install-files uninstall

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

install: build install-files

install-files:
	install -Dm755 target/release/shr $(DESTDIR)$(PREFIX)/bin/shr
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth
	ln -sfn shr $(DESTDIR)$(PREFIX)/bin/synth-player
	ln -sfn shr $(DESTDIR)$(PREFIX)/bin/shs
	install -Dm755 scripts/setup.sh $(DESTDIR)$(PREFIX)/bin/shr-setup
	install -Dm755 scripts/audio-performance.sh $(DESTDIR)$(PREFIX)/bin/shr-audio-tune
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth-setup
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/presets/synthv1
	set -e; while IFS= read -r preset; do \
	  install -m644 "presets/synthv1/$$preset" $(DESTDIR)$(PREFIX)/share/shsynth/presets/synthv1/; \
	done < presets/synthv1/cleared-presets.txt
	install -m644 presets/synthv1/cleared-presets.txt $(DESTDIR)$(PREFIX)/share/shsynth/presets/synthv1/
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/config
	install -m644 config/*.conf $(DESTDIR)$(PREFIX)/share/shsynth/config/
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/midi-devices
	install -m644 midi-devices/*.json $(DESTDIR)$(PREFIX)/share/shsynth/midi-devices/
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/controller-profiles
	install -m644 controller-profiles/*.json $(DESTDIR)$(PREFIX)/share/shsynth/controller-profiles/
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/drum-patterns
	install -m644 drum-patterns/*.shdrum $(DESTDIR)$(PREFIX)/share/shsynth/drum-patterns/
	install -m644 drum-patterns/*.shrdrums $(DESTDIR)$(PREFIX)/share/shsynth/drum-patterns/
	install -d $(DESTDIR)$(PREFIX)/share/doc/shsynth/images
	install -m644 LICENSE THIRD_PARTY.md README.md $(DESTDIR)$(PREFIX)/share/doc/shsynth/
	install -m644 docs/*.md $(DESTDIR)$(PREFIX)/share/doc/shsynth/
	install -m644 docs/images/*.html docs/images/*.jpg docs/images/*.png $(DESTDIR)$(PREFIX)/share/doc/shsynth/images/
	install -d $(DESTDIR)$(PREFIX)/share/doc/shsynth/menu
	install -m644 docs/menu/*.md $(DESTDIR)$(PREFIX)/share/doc/shsynth/menu/
	install -d $(DESTDIR)$(PREFIX)/share/doc/shsynth/images/menu
	install -m644 docs/images/menu/*.png $(DESTDIR)$(PREFIX)/share/doc/shsynth/images/menu/

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth $(DESTDIR)$(PREFIX)/bin/shr
	rm -f $(DESTDIR)$(PREFIX)/bin/synth-player $(DESTDIR)$(PREFIX)/bin/shs
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth-setup $(DESTDIR)$(PREFIX)/bin/shr-setup
	rm -f $(DESTDIR)$(PREFIX)/bin/shr-audio-tune
	rm -rf $(DESTDIR)$(PREFIX)/share/shsynth
	rm -rf $(DESTDIR)$(PREFIX)/share/doc/shsynth
