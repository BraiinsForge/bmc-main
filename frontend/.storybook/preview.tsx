import type { Preview } from '@storybook/react';
import { MINIMAL_VIEWPORTS } from 'storybook/viewport';

import Container from './Container';
import THEME from '../src/styles/theme';

// https://github.com/storybookjs/storybook/pull/24555
// https://github.com/storybookjs/storybook/issues/22452
// @ts-ignore: Fugly hack to work around a regression in storybook.
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
