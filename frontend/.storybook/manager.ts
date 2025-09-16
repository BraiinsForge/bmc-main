import { addons } from 'storybook/manager-api';
import { create as createTheme } from 'storybook/theming/create';

/** @see https://storybook.js.org/docs/7.0/react/configure/features-and-behavior */
addons.setConfig({
    theme: createTheme({
        base: 'dark',
        brandTitle: 'Braiins',
        brandUrl: 'https://braiins.com',
    }),

    isFullscreen: false,
    showNav: true, // Display panel that shows a list of stories
    showPanel: true, // Display panel that shows addon configurations
    panelPosition: 'bottom',
    sidebarAnimations: true,
    enableShortcuts: true,
    showToolbar: true,

    sidebar: {
        showRoots: true,
        collapsedRoots: [
            //
            'components',
            'layouts',
            'init',
            'accounts',
            'alarms',
            'display',
            'network',
            'settings',
        ],
    },
});
