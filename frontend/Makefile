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
	yarn run prettier './**/*.scss' --write

.PHONY: lint
lint:
	yarn run biome check --max-diagnostics=100 .
	yarn run stylelint '**/*.scss' --color --quiet --formatter verbose --allow-empty-input --config stylelint.config.cjs

.PHONY: test
test:
	yarn run tsc --diagnostics --pretty --project ./tsconfig.json
	yarn run vitest run

#
# Build
#

.PHONY: check
check: format lint test

.PHONY: build
build:
	yarn run rsbuild build

.PHONY: dev
dev:
	yarn run rsbuild dev --open

.PHONY: serve
serve:
	cd ..; \
	cargo run --package bmc-mock -- \
		--address="0.0.0.0:6070" \
		--www-path="$(shell realpath ./dist)" \
		--mockfs-path="$(shell realpath ./@mockfs)"

#
# Develop
#

.PHONY: storybook-start
storybook-start:
	yarn run storybook dev \
	  --no-open \
	  --port=6006 \
	  --exact-port \
	  --disable-telemetry
