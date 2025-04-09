import type { StorybookConfig } from 'storybook-react-rsbuild';

const config: StorybookConfig = {
    framework: 'storybook-react-rsbuild',
    stories: ['../src/**/*.stories.tsx'],
    addons: [
        {
            name: '@storybook/addon-essentials',
            options: { docs: false },
        },
    ],
};

export default config;
