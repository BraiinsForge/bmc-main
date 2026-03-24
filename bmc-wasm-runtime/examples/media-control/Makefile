# Media Control Widget — test targets
#
# No external dependencies — uses built-in Python UPnP renderer.

TOOLS := tools/dlna-push.py
PORT  ?= 49494
NAME  ?= Deck Test
HOST  ?= 127.0.0.1:$(PORT)

.PHONY: serve
serve:
	python3 $(TOOLS) --port $(PORT) --name "$(NAME)" --example 0

.PHONY: serve-music
serve-music:
	python3 $(TOOLS) --port $(PORT) --name "$(NAME)" --example 1

.PHONY: serve-classical
serve-classical:
	python3 $(TOOLS) --port $(PORT) --name "$(NAME)" --example 2

.PHONY: serve-radio
serve-radio:
	python3 $(TOOLS) --port $(PORT) --name "$(NAME)" --example 3

.PHONY: serve-url
serve-url:
	@test -n "$(URL)" || { echo "usage: make serve-url URL=https://..."; exit 1; }
	python3 $(TOOLS) --port $(PORT) --name "$(NAME)" $(URL)

.PHONY: spawn
spawn:
	python3 $(TOOLS) --port $(PORT) --name "$(NAME)"

.PHONY: push
push:
	@test -n "$(URL)" || { echo "usage: make push URL=https://..."; exit 1; }
	python3 $(TOOLS) $(HOST) $(URL)

.PHONY: status
status:
	python3 $(TOOLS) $(HOST) --status
