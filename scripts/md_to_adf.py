#!/usr/bin/env python3
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

"""Translate CommonMark into Atlassian Document Format, on stdout.

Parses with markdown-it-py, what mdformat normalises these files with, so the
conversion agrees with the formatter. An unmapped construct raises: a ticket
that quietly loses a paragraph is worse than one that fails to upload.

Run through uv, which supplies markdown-it-py:

  uv run scripts/md_to_adf.py FILE [--skip-title]
  uv run scripts/md_to_adf.py -
"""

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from markdown_it import MarkdownIt
from markdown_it.tree import SyntaxTreeNode

# markdown-it node types carrying no content of their own.
IGNORED = frozenset({'inline'})


class Unmapped(Exception):
    """A construct with no ADF equivalent implemented here."""


def convert(markdown: str, *, skip_title: bool) -> dict[str, Any]:
    # Without the GFM table extension a table parses
    # as pipe characters, mangled rather than raised on.
    parser = MarkdownIt('commonmark').enable('table')
    tree = SyntaxTreeNode(parser.parse(markdown))
    blocks = list(tree.children)

    if skip_title and blocks and blocks[0].type == 'heading' and blocks[0].tag == 'h1':
        blocks = blocks[1:]

    return {
        'version': 1,
        'type': 'doc',
        'content': [node for block in blocks for node in block_nodes(block)],
    }


def block_nodes(node: SyntaxTreeNode) -> list[dict[str, Any]]:
    """One block-level markdown node as the ADF nodes it becomes."""
    match node.type:
        case 'heading':
            return [
                {
                    'type': 'heading',
                    'attrs': {'level': int(node.tag[1:])},
                    'content': inline_nodes(node),
                }
            ]
        case 'paragraph':
            content = inline_nodes(node)
            # An empty paragraph is valid ADF, but only adds a blank line.
            return [{'type': 'paragraph', 'content': content}] if content else []
        case 'bullet_list':
            return [{'type': 'bulletList', 'content': children_blocks(node)}]
        case 'ordered_list':
            return [{'type': 'orderedList', 'content': children_blocks(node)}]
        case 'list_item':
            return [{'type': 'listItem', 'content': children_blocks(node)}]
        case 'blockquote':
            return [{'type': 'blockquote', 'content': children_blocks(node)}]
        case 'fence' | 'code_block':
            attrs = {'language': node.info.strip()} if node.info.strip() else {}
            return [
                {
                    'type': 'codeBlock',
                    'attrs': attrs,
                    'content': [{'type': 'text', 'text': node.content.rstrip('\n')}],
                }
            ]
        case 'hr':
            return [{'type': 'rule'}]
        case 'table':
            return [
                {
                    'type': 'table',
                    'attrs': {'isNumberColumnEnabled': False, 'layout': 'default'},
                    'content': children_blocks(node),
                }
            ]
        case 'thead' | 'tbody':
            # Grouping only; ADF hangs rows straight off the table.
            return children_blocks(node)
        case 'tr':
            return [{'type': 'tableRow', 'content': children_blocks(node)}]
        case 'th' | 'td':
            cell = 'tableHeader' if node.type == 'th' else 'tableCell'
            return [
                {
                    'type': cell,
                    'attrs': {},
                    'content': [{'type': 'paragraph', 'content': inline_nodes(node)}],
                }
            ]
        case _:
            raise Unmapped(f'block node {node.type!r}')


def children_blocks(node: SyntaxTreeNode) -> list[dict[str, Any]]:
    return [adf for child in node.children for adf in block_nodes(child)]


def inline_nodes(node: SyntaxTreeNode) -> list[dict[str, Any]]:
    """The inline content of a block, flattened with its marks resolved."""
    inline = next((c for c in node.children if c.type == 'inline'), None)
    if inline is None:
        return []
    return marked_nodes(inline.children, marks=[])


def marked_nodes(
    nodes: list[SyntaxTreeNode], *, marks: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for node in nodes:
        match node.type:
            case 'text':
                append_text(out, node.content, marks)
            case 'code_inline':
                append_text(out, node.content, [*marks, {'type': 'code'}])
            case 'strong':
                out.extend(
                    marked_nodes(node.children, marks=[*marks, {'type': 'strong'}])
                )
            case 'em':
                out.extend(marked_nodes(node.children, marks=[*marks, {'type': 'em'}]))
            case 's':
                out.extend(
                    marked_nodes(node.children, marks=[*marks, {'type': 'strike'}])
                )
            case 'link':
                href = node.attrs['href']
                link = {'type': 'link', 'attrs': {'href': href}}
                linked = marked_nodes(node.children, marks=[*marks, link])
                # A link mark needs a text node to sit on,
                # so `[](url)` would drop the href on the floor.
                if not linked:
                    raise Unmapped(f'link with no text ({href})')
                out.extend(linked)
            case 'softbreak':
                # ADF has no soft break, and a hardBreak would be a real newline.
                append_text(out, ' ', marks)
            case 'hardbreak':
                out.append({'type': 'hardBreak'})
            case _ if node.type in IGNORED:
                out.extend(marked_nodes(node.children, marks=marks))
            case _:
                raise Unmapped(f'inline node {node.type!r}')
    return merge_adjacent(out)


def append_text(
    out: list[dict[str, Any]], text: str, marks: list[dict[str, Any]]
) -> None:
    node = text_node(text, marks)
    if node is not None:
        out.append(node)


def text_node(text: str, marks: list[dict[str, Any]]) -> dict[str, Any] | None:
    # ADF requires a non-empty `text`; markdown-it emits empty tokens.
    if not text:
        return None
    node: dict[str, Any] = {'type': 'text', 'text': text}
    if marks:
        node['marks'] = marks
    return node


def merge_adjacent(nodes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Join neighbouring text nodes with identical marks, which softbreaks split."""
    merged: list[dict[str, Any]] = []
    for node in nodes:
        previous = merged[-1] if merged else None
        if (
            previous is not None
            and previous['type'] == 'text'
            and node['type'] == 'text'
            and previous.get('marks') == node.get('marks')
        ):
            previous['text'] += node['text']
        else:
            merged.append(node)
    return merged


def main() -> int:
    parser = argparse.ArgumentParser(
        description='Translate CommonMark into Atlassian Document Format.'
    )
    parser.add_argument('file', help="markdown file, or '-' for stdin")
    parser.add_argument(
        '--skip-title',
        action='store_true',
        help='drop a leading level-1 heading',
    )
    args = parser.parse_args()

    # Pinned to UTF-8, or the locale decides and em dashes break it.
    markdown = (
        sys.stdin.buffer.read().decode('utf-8')
        if args.file == '-'
        else Path(args.file).read_text(encoding='utf-8')
    )
    try:
        document = convert(markdown, skip_title=args.skip_title)
    except Unmapped as unmapped:
        print(f'{args.file}: no ADF mapping for {unmapped}', file=sys.stderr)
        return 1

    body = json.dumps(document, indent=2, ensure_ascii=False)
    sys.stdout.buffer.write(f'{body}\n'.encode('utf-8'))
    return 0


if __name__ == '__main__':
    sys.exit(main())
