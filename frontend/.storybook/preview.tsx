// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import type { Preview } from '@storybook/react';
import { MINIMAL_VIEWPORTS } from 'storybook/viewport';

import Container from './Container';
import THEME from '../src/styles/theme';

// https://github.com/storybookjs/storybook/pull/24555
// https://github.com/storybookjs/storybook/issues/22452
// @ts-expect-error: Fugly hack to work around a regression in storybook.
BigInt.prototype.toJSON = function () {
    return this.toString();
};

export default {
    // Global defaults for story parameters
    parameters: {
        // Enable expanded mode
        controls: {
            expanded: true,
            hideNoControlsWarning: true,
            matchers: {
                color: /(background|color|fill|stroke)$/i,
                date: /Date$/,
            },
            disableSaveFromUI: true,
        },
        viewport: {
            viewports: MINIMAL_VIEWPORTS,
        },

        // This is needed because some stories render modals,
        // which crews-up the "docs" tab rendering for all stories.
        docs: { inlineStories: false },
    },

    globalTypes: {
        theme: {
            name: 'theme',
            description: 'Global theme for components',
            table: { defaultValue: { summary: 'dark' } },
            toolbar: {
                title: 'Theme',
                icon: 'eye',
                items: Object.keys(THEME).map(key => ({ value: key, title: `${key} (${THEME[key]})` })),
            },
        },
    },

    // Supplies all globally used contexts so that each story doesn't need to do that
    decorators: [
        function wrap(story, context) {
            return (
                <Container
                    story={story}
                    theme={context.globals.theme || THEME.dark}
                    i18n={null /* languagesData[context.globals.language] */}
                />
            );
        },
    ],

    argTypes: {
        intl: { table: { disable: true } },
    },
} satisfies Preview;
