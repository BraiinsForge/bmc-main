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
	cargo run --package bmc-mock -- \
		--address="0.0.0.0:6070" \
		--system-password="root" \
		--www-path="$(shell realpath ./dist)" \
		--mockfs-path="$(shell realpath ./@mockfs)" $(EXTRA_ARGS)

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

.PHONY: minify-png
minify-png:
	@which oxipng > /dev/null || (echo "Error: oxipng is not installed" && exit 1)
	oxipng --zopfli --fast --alpha --strip=safe --opt=max --recursive --preserve ./src

.PHONY: minify-svg
minify-svg:
	yarn run svgo --recursive --folder ./src

.PHONY: minify-images
minify-images: minify-png minify-svg

.PHONY: intl-extract
intl-extract:
	yarn run formatjs extract './src/**/*.{ts,tsx}' --ignore='**/*.d.ts' --out-file lang/en.json

.PHONY: nix-yarn-files
nix-yarn-files:
	# "build" the yarn files derivation to check
	# if the hard-coded hash is correct
	nix build -L '.#yarnFiles'
	rm -rf ./result
