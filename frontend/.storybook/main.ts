// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

import type { StorybookConfig } from 'storybook-react-rsbuild';

const config: StorybookConfig = {
    framework: 'storybook-react-rsbuild',
    stories: ['../src/**/*.stories.tsx'],
    addons: [],
    features: {
        // The rsbuild builder doesn't implement Storybook 10's change-detection hook,
        // so the feature only logs "builder does not support change detection"
        // and adds nothing. Turn it off to silence the warning.
        changeDetection: false,
        interactions: false,
    },
    rsbuildFinal: async config => {
        // Fix: Remove the main app entries to prevent conflict with Storybook's iframe.html
        // The main rsbuild.config.ts defines multiple entries (index, index-connect) with html: true
        // which causes Storybook to try generating multiple iframe.html files, resulting in:
        // "Multiple assets emit different content to the same filename iframe.html"
        // We need to remove only the app entries while preserving storybook's own entries.
        if (config.source?.entry) {
            const entries = config.source.entry;
            // Remove app entries but keep any storybook entries
            if (typeof entries === 'object') {
                delete entries.index;
                delete entries['index-connect'];
            }
        }
        return config;
    },
};

export default config;
