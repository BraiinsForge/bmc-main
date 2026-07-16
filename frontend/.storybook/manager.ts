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
