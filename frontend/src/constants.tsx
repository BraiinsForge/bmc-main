export const URLS = {
    defaultScreen: '/display',

    auth: {
        login: '/login',
    },

    pages: {
        initSetup: '/init_setup',

        display: {
            list: '/display',
            combined: {
                path: '/display/:id',
                getHref: (id: string) => `/display/${id}`,
            },
        },
        settings: '/settings',
        alarms: '/alarms',
        priceAlerts: '/price-alerts',
        notifications: '/notifications',
        network: '/network',
        api: '/api',
        buyButton: '/buy-button',
    },

    external: {
        academy: 'https://academy.braiins.com/',
    },
} as const;
