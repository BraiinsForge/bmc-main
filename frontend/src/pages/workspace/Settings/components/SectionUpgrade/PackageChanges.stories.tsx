import type { Meta } from '@storybook/react';

import type * as pb from '@/proto';
import { PackageChanges as Component } from './PackageChanges';

const changes: pb.PackageChange[] = [
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'bmc',
        category: 'core',
        versionFrom: '24.08.1',
        versionTo: '24.09.4',
        changelog: '## Coordinator\n\n- faster scene switching\n- fixed a rare memory leak on wake',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'clock-widget',
        category: 'widget',
        versionFrom: '1.4.0',
        versionTo: '1.5.0',
        changelog: '- new analog face\n- DST edge-case fix',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'weather-widget',
        category: 'widget',
        versionTo: '2.0.0',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'braiins-cli',
        category: 'dev',
        versionFrom: '1.1.0',
        versionTo: '1.2.0',
        changelog: 'Minor CLI polish.',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'nix-support',
        versionTo: '0.3.0',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'legacy-ticker',
        category: 'widget',
        versionFrom: '0.4.2',
    },
];

export default {
    title: 'settings/components/PackageChanges',
    component: Component,
    decorators: [
        Story => (
            <div className="ui-box" style={{ maxWidth: 560 }}>
                <Story />
            </div>
        ),
    ],
} satisfies Meta<typeof Component>;

export function PackageChanges() {
    return <Component changes={changes} />;
}
