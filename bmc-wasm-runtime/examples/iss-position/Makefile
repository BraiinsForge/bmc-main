TOOLS_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))tools

.PHONY: render
render: ## Render Natural Earth dark texture (1024x512)
	uv run --project $(TOOLS_DIR) $(TOOLS_DIR)/texture_render.py

.PHONY: download
download: ## Download all reference textures from the catalog
	uv run --project $(TOOLS_DIR) $(TOOLS_DIR)/texture_download.py

.PHONY: redownload
redownload: ## Force re-download all reference textures
	uv run --project $(TOOLS_DIR) $(TOOLS_DIR)/texture_download.py --force

.PHONY: preview
preview: ## Start local texture comparison viewer
	uv run --project $(TOOLS_DIR) $(TOOLS_DIR)/texture_preview.py
	$(MAKE) -C ../../.. format

.PHONY: lint
lint: ## Type-check tools scripts with their local venv
	uvx ty check --project $(TOOLS_DIR)
