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

"""Cover the ADF shapes Jira rejects and the ones it silently renders wrong."""

import importlib.util
import sys
from pathlib import Path
from typing import Any

import pytest

_SPEC = importlib.util.spec_from_file_location(
    'md_to_adf', Path(__file__).resolve().parents[1] / 'md_to_adf.py'
)
assert _SPEC is not None and _SPEC.loader is not None, (
    'BUG: md_to_adf.py must be importable'
)
md_to_adf = importlib.util.module_from_spec(_SPEC)
sys.modules['md_to_adf'] = md_to_adf
_SPEC.loader.exec_module(md_to_adf)


def convert(markdown: str, *, skip_title: bool = False) -> dict[str, Any]:
    return md_to_adf.convert(markdown, skip_title=skip_title)


def text_nodes(document: dict[str, Any]) -> list[dict[str, Any]]:
    """Every text node in the tree, at any depth."""
    found: list[dict[str, Any]] = []

    # An ADF tree is heterogeneous JSON, so the recursion is genuinely untyped.
    def walk(node: Any) -> None:
        if isinstance(node, dict):
            if node.get('type') == 'text':
                found.append(node)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(document)
    return found


def test_document_is_an_adf_doc():
    document = convert('Plain.')

    assert document['type'] == 'doc'
    assert document['version'] == 1


def test_heading_level_comes_from_the_tag():
    document = convert('### Third')

    assert document['content'][0] == {
        'type': 'heading',
        'attrs': {'level': 3},
        'content': [{'type': 'text', 'text': 'Third'}],
    }


def test_no_text_node_is_ever_empty():
    """ADF requires a non-empty `text`, and markdown-it emits empty tokens."""
    document = convert('**Jira:** _(fill in)_')

    assert all(node['text'] for node in text_nodes(document))


def test_a_softbreak_becomes_a_space_not_a_line_break():
    """Prose here wraps at clause boundaries; a hardBreak would make each wrap real."""
    document = convert('one\ntwo')

    assert text_nodes(document) == [{'type': 'text', 'text': 'one two'}]


def test_adjacent_runs_with_equal_marks_merge():
    document = convert('plain `code` more')

    assert [node['text'] for node in text_nodes(document)] == [
        'plain ',
        'code',
        ' more',
    ]


def test_marks_nest_through_a_link():
    document = convert('[**bold link**](https://example.test)')

    (node,) = text_nodes(document)
    assert node['text'] == 'bold link'
    assert {'type': 'strong'} in node['marks']
    assert {'type': 'link', 'attrs': {'href': 'https://example.test'}} in node['marks']


def test_bullet_list_wraps_items_in_paragraphs():
    document = convert('- one\n- two')

    bullet = document['content'][0]
    assert bullet['type'] == 'bulletList'
    assert [item['type'] for item in bullet['content']] == ['listItem', 'listItem']
    assert bullet['content'][0]['content'][0]['type'] == 'paragraph'


def test_fence_keeps_its_language():
    document = convert('```python\nx = 1\n```')

    assert document['content'][0] == {
        'type': 'codeBlock',
        'attrs': {'language': 'python'},
        'content': [{'type': 'text', 'text': 'x = 1'}],
    }


def test_skip_title_drops_only_a_leading_h1():
    document = convert('# Title\n\nBody.', skip_title=True)

    assert [block['type'] for block in document['content']] == ['paragraph']


def test_skip_title_leaves_a_later_heading_alone():
    document = convert('# Title\n\n## Section', skip_title=True)

    assert document['content'][0]['attrs']['level'] == 2


def test_a_table_becomes_an_adf_table():
    """GFM tables are off in the commonmark preset, where they mangle into pipe text."""
    document = convert('| a | b |\n| - | - |\n| 1 | 2 |')

    table = document['content'][0]
    assert table['type'] == 'table'
    assert [row['type'] for row in table['content']] == ['tableRow', 'tableRow']
    assert [cell['type'] for cell in table['content'][0]['content']] == [
        'tableHeader',
        'tableHeader',
    ]
    assert [cell['type'] for cell in table['content'][1]['content']] == [
        'tableCell',
        'tableCell',
    ]


def test_an_empty_table_cell_survives():
    document = convert('| a | b |\n| - | - |\n| 1 |   |')

    empty = document['content'][0]['content'][1]['content'][1]
    assert empty == {
        'type': 'tableCell',
        'attrs': {},
        'content': [{'type': 'paragraph', 'content': []}],
    }


def test_an_unmapped_construct_raises_rather_than_dropping_content():
    """A ticket that silently loses content is worse than one that fails to upload."""
    with pytest.raises(md_to_adf.Unmapped):
        convert('![alt](https://example.test/x.png)')


def test_a_link_with_no_text_raises_rather_than_dropping_its_href():
    with pytest.raises(md_to_adf.Unmapped):
        convert('[](https://example.test)')
