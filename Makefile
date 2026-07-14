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
	install -Dm755 synth-player $(DESTDIR)$(PREFIX)/bin/synth-player
	ln -sfn synth-player $(DESTDIR)$(PREFIX)/bin/shs
	install -Dm755 scripts/setup.sh $(DESTDIR)$(PREFIX)/bin/shr-setup
	install -Dm755 scripts/audio-performance.sh $(DESTDIR)$(PREFIX)/bin/shr-audio-tune
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth-setup
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/presets/synthv1
	for preset in \
	  "Velvet Tines.synthv1" \
	  "Deep Sub.synthv1" "Liquid Acid.synthv1" "Rubber Circuit.synthv1" "Compact Bass.synthv1" \
	  "Mono Pulse Lead.synthv1" "PWM Horizon.synthv1" "Glass Saw Lead.synthv1" \
	  "Warm Cloud.synthv1" "Dark Canopy.synthv1" "Shimmer Veil.synthv1" \
	  "Copper Pluck.synthv1" "Reed Pluck.synthv1" "Silver Bell.synthv1" "Soft Chime.synthv1" \
	  "Drawbar Glow.synthv1" "Hollow Organ.synthv1" \
	  "Low Orbit Drone.synthv1" "Frozen Drone.synthv1" "Dust Delay.synthv1" "Restrained Sweep.synthv1"; do \
	  install -m644 "presets/synthv1/$$preset" $(DESTDIR)$(PREFIX)/share/shsynth/presets/synthv1/; \
	done
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/config
	install -m644 config/*.conf $(DESTDIR)$(PREFIX)/share/shsynth/config/
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/midi-devices
	install -m644 midi-devices/*.json $(DESTDIR)$(PREFIX)/share/shsynth/midi-devices/
	install -d $(DESTDIR)$(PREFIX)/share/shsynth/controller-profiles
	install -m644 controller-profiles/*.json $(DESTDIR)$(PREFIX)/share/shsynth/controller-profiles/

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth $(DESTDIR)$(PREFIX)/bin/shr
	rm -f $(DESTDIR)$(PREFIX)/bin/synth-player $(DESTDIR)$(PREFIX)/bin/shs
	rm -f $(DESTDIR)$(PREFIX)/bin/shsynth-setup $(DESTDIR)$(PREFIX)/bin/shr-setup
	rm -f $(DESTDIR)$(PREFIX)/bin/shr-audio-tune
	rm -rf $(DESTDIR)$(PREFIX)/share/shsynth
