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

    api: {
        supportArchive: '/api/get_support_archive',
    },

    external: {
        academy: 'https://academy.braiins.com/',
        pool: {
            accessProfiles: 'https://pool.braiins.com/settings/access',
        },
        widgetsDirectory: 'https://widgets.braiinsforge.com',
    },
} as const;
