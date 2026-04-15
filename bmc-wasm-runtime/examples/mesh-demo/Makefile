DIR := $(shell pwd)
BLENDER := $(shell \
	if command -v blender >/dev/null 2>&1; then \
		echo blender; \
	elif command -v flatpak >/dev/null 2>&1; then \
		echo "flatpak run org.blender.Blender"; \
	elif command -v snap >/dev/null 2>&1; then \
		echo "snap run blender"; \
	fi)

# PIL is needed for font rendering on numbered dice textures.
# Use nix-shell to get Pillow if not already available.
PYTHON_PIL := nix-shell -p python3Packages.pillow --run

DICE := D4 D6 D8 D10 D12 D20

.PHONY: .assert
.assert:
	@if [ -z "$(BLENDER)" ]; then \
		echo "Error: Blender not found in PATH, flatpak, or snap." >&2; \
		exit 1; \
	fi

.PHONY: suzanne $(DICE)
suzanne: .assert
	$(BLENDER) --background --python $(DIR)/tools/suzanne.py -- $(DIR)/assets/suzanne.glb

# D6 uses pips (no font rendering needed).
D6: .assert
	$(BLENDER) --background --python $(DIR)/tools/$@.py -- $(DIR)/assets/$@.glb

# Numbered dice: pre-generate textures with PIL (BraiinsSans-Bold font),
# since Blender's bundled Python lacks PIL.
$(filter-out D6,$(DICE)): .assert
	rm -f $(DIR)/assets/$@.*.png
	$(PYTHON_PIL) 'python3 $(DIR)/tools/$@.py --reset-texture $(DIR)/assets/$@'
	$(BLENDER) --background --python $(DIR)/tools/$@.py -- $(DIR)/assets/$@.glb

.PHONY: generate
generate: suzanne $(DICE)

.PHONY: $(addprefix reset-,$(DICE))
$(addprefix reset-,$(DICE)):
	$(eval DIE = $(patsubst reset-%,%,$@))
	rm -f $(DIR)/assets/$(DIE).*.png
	$(PYTHON_PIL) 'python3 $(DIR)/tools/$(DIE).py --reset-texture $(DIR)/assets/$(DIE)'
