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
        accounts: '/accounts',
    },

    external: {
        academy: 'https://academy.braiins.com/',
        pool: {
            accessProfiles: 'https://pool.braiins.com/settings/access',
        },
    },
} as const;
