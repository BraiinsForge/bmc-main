import type { StorybookConfig } from 'storybook-react-rsbuild';

const config: StorybookConfig = {
    framework: 'storybook-react-rsbuild',
    stories: ['../src/**/*.stories.tsx'],
    addons: [],
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
