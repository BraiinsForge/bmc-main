.DEFAULT_GOAL: configure

#
# Setup
#

.PHONY: gen
gen:
	yarn run buf generate --verbose
	$(MAKE) format

.PHONY: configure
configure:
	# SDKs are only applicable when PnP is active
	# yarn dlx @yarnpkg/sdks base

#
# QA
#

.PHONY: format
format:
	yarn run biome migrate --write
	yarn run biome format --write .
	yarn run prettier './**/*.scss' --log-level="warn" --write

.PHONY: lint
lint:
	yarn run biome check --max-diagnostics=100 .
	yarn run stylelint '**/*.scss' --color --quiet --formatter string --allow-empty-input --config stylelint.config.cjs

.PHONY: test
test:
	yarn run tsc --diagnostics --pretty --project ./tsconfig.json
	yarn run rstest run

#
# Build
#

.PHONY: validate
validate: format lint test

.PHONY: build
build:
	yarn run rsbuild build

.PHONY: dev
dev:
	yarn run rsbuild dev

.PHONY: serve serve-factory

serve: EXTRA_ARGS ?= ;
serve:
	echo $(FACTORY_DEFAULT)
	cd ..; \
	cargo run \
		--features=winit-skia \
		--package bmc-mock -- \
		--address="0.0.0.0:6070" \
		--system-password="root" \
		--www-path="$(shell realpath ./dist)" \
		--sounds-dir="$(shell realpath ./@data/sounds)" \
		--mockfs-path="$(shell realpath ./@data/mockfs)" $(EXTRA_ARGS)

serve-factory:
	EXTRA_ARGS='--factory-default' $(MAKE) serve

#
# Develop
#

.PHONY: storybook-start
storybook-start:
	yarn run storybook dev \
	  --no-open \
	  --port=6005 \
	  --exact-port \
	  --disable-telemetry

.PHONY: minify-jpg
minify-jpg:
	@command -v jpegoptim >/dev/null 2>&1 || { echo "❌ Error: jpegoptim is not installed"; exit 1; }
	@find . \
		\( -path '*/.*' -o -type d -name build \) -prune -o \
		\( -type f \( -iname '*.jpg' -o -iname '*.jpeg' \) \) \
		-exec jpegoptim --max=70 --strip-all --threshold=5 {} \;

.PHONY: minify-png
minify-png:
	@which oxipng > /dev/null || (echo "Error: oxipng is not installed" && exit 1)
	oxipng --zopfli --fast --alpha --strip=safe --opt=max --recursive --preserve ./src

.PHONY: minify-svg
minify-svg:
	yarn run svgo --recursive --folder ./src

.PHONY: minify-images
minify-images: minify-jpg minify-png minify-svg

.PHONY: intl-extract
intl-extract:
	yarn run formatjs extract './src/**/*.{ts,tsx}' --ignore='**/*.d.ts' --out-file lang/en.json

PHONY: nix-update-hash
nix-update-hash:
	./bin/nix-update-hash.ts

#
# Image Minification
#
