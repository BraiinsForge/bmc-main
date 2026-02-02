# Makefile for self-contained ffmpeg wrapper with embedded binary
#
# Usage:
#   make -f makefile.embedded FFMPEG_BIN=../ffmpeg-7.1/ffmpeg cross
#   make -f makefile.embedded clean
#
# This creates a single binary with ffmpeg embedded inside.

TARGET = bmc-video-play
SRC = src/main.c
EMBEDDED_HEADER = src/ffmpeg_embedded.h

# Path to the ffmpeg binary to embed
FFMPEG_BIN ?= ../ffmpeg-7.1/ffmpeg

# Cross-compilation settings
CROSS_PREFIX ?= armv7-linux-musleabihf-
CC_cross ?= $(CROSS_PREFIX)gcc

CFLAGS = -Wall -Wextra -O2 -static -pthread -I./src
LDFLAGS = -static -pthread -lm

.PHONY: all cross clean generate-header

all: bmc-video-play

# Generate C header with embedded ffmpeg binary
$(EMBEDDED_HEADER): $(FFMPEG_BIN)
	@echo "==> Generating embedded ffmpeg header from $(FFMPEG_BIN)"
	@echo "// Auto-generated - do not edit" > $(EMBEDDED_HEADER)
	@echo "// Embedded ffmpeg binary" >> $(EMBEDDED_HEADER)
	@echo "#ifndef FFMPEG_EMBEDDED_H" >> $(EMBEDDED_HEADER)
	@echo "#define FFMPEG_EMBEDDED_H" >> $(EMBEDDED_HEADER)
	@echo "#include <stddef.h>" >> $(EMBEDDED_HEADER)
	@echo "" >> $(EMBEDDED_HEADER)
	@echo "static const unsigned char ffmpeg_bin[] = {" >> $(EMBEDDED_HEADER)
	xxd -i < $(FFMPEG_BIN) | head -n -1 >> $(EMBEDDED_HEADER)
	@echo "};" >> $(EMBEDDED_HEADER)
	@echo "" >> $(EMBEDDED_HEADER)
	@echo "static const size_t ffmpeg_bin_len = sizeof(ffmpeg_bin);" >> $(EMBEDDED_HEADER)
	@echo "#endif" >> $(EMBEDDED_HEADER)
	@echo "==> Generated $(EMBEDDED_HEADER) (ffmpeg size: $$(stat -c%s $(FFMPEG_BIN)) bytes)"

generate-header: $(EMBEDDED_HEADER)

$(TARGET): $(EMBEDDED_HEADER)
	@echo "==> Building embedded wrapper (cross)"
	$(CC_cross) $(CFLAGS) -o $(TARGET) $(SRC) $(LDFLAGS)
	@echo ""
	@echo "==> Build complete:"
	@ls -lh $(TARGET)
	@file $(TARGET)
	@echo ""
	@echo "This single binary contains ffmpeg embedded inside."

clean:
	rm -f $(TARGET) $(EMBEDDED_HEADER)
