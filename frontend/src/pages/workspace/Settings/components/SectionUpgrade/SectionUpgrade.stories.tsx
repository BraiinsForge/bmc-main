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

import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import * as gen from '@/mocks';
import type * as pb from '@/proto';
import { UpgradeDisruption, FirmwareUpgradePhase, PackageUpgradePhase } from '@/proto';
import { SectionUpgrade as Component, type SectionUpgradeProps } from './SectionUpgrade';

export default {
    title: 'settings/components/SectionUpgrade',
    component: Component,
    args: {
        automaticUpgrades: {
            value: true,
            disabled: false,
            onChange: action('automaticUpdates.onChange'),
        },
        versionCurrent: '24.04.1',
        status: null,
        errors: [],

        onStartUpgrade: action('onStartUpgrade'),
        onCheckUpdates: action('onCheckUpdates'),
    } satisfies SectionUpgradeProps,
} satisfies Meta<SectionUpgradeProps>;

const upgradeInfo: pb.CheckForUpgradeResponse = {
    $typeName: 'braiins.bmc.web.CheckForUpgradeResponse',
    upgradeId: 'upgrade-firmware-0',
    firmware: {
        $typeName: 'braiins.bmc.web.FirmwareUpgrade',
        version: '24.09.4',
        hash: 'db777c17acb949bcba3a69ba12875857',
        releaseDate: gen.protoTimestamp.days(-5),
        fileSizeBytes: 365_000_000n,
        description:
            'Introducing Braiins OS version 24.08! This update brings initial support for Antminer S19 XP Hydro and Antminer T21 Zynq/Xilinx control board, along with several enhancements.\n\n## Antminer S21 & S19\n\n- Support of all BOS features now available for Antminer S19 XP Hydro\n- Support of all BOS features now available for Antminer T21 Zynq/Xilinx control board',
        previousReleases: [
            {
                $typeName: 'braiins.bmc.web.ReleaseInfo',
                version: '2024-07-25-0-e346502d-24.06.1-plus',
                description:
                    'Bugfix release introducing an important fix\n\n## Antminer Family\n\n- Fixed a bug related to BeagleBone Black installation and Zynq/Xilinx downgrade\n',
            },
            {
                $typeName: 'braiins.bmc.web.ReleaseInfo',
                version: '2024-07-12-0-fc9fe388-24.06-plus',
                description:
                    'Introducing Braiins OS version 24.06, featuring significantly improved tuning time, a few features for Braiins Mini Miner, and more.\n\n## Antminer Family\n\n- Improved tuning time for 1366BM/1368BM chips\n- Better DNS caching to reduce network bandwidth\n',
            },
        ],
    },
    disruption: UpgradeDisruption.REBOOT,
};
const packageUpgradeInfo: pb.CheckForUpgradeResponse = {
    $typeName: 'braiins.bmc.web.CheckForUpgradeResponse',
    upgradeId: 'upgrade-packages-0',
    packages: {
        $typeName: 'braiins.bmc.web.PackageUpgradePlan',
        bmcVersion: '24.09.4',
        downloadSizeBytes: 45_000_000n,
        bmcChangelog: '## Braiins Deck 24.09.4\n\n- new widgets picker\n- faster scene switching',
        changes: [
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
        ],
    },
    disruption: UpgradeDisruption.APP_RESTART,
};

export function Empty(args: SectionUpgradeProps) {
    return <Component {...args} />;
}

export function Idle(args: SectionUpgradeProps) {
    return <Component {...args} status={{ kind: 'idle', upgradeInfo: null }} />;
}

export function Checking(args: SectionUpgradeProps) {
    return <Component {...args} status={{ kind: 'checking-upgrade', upgradeInfo: null }} />;
}

export function UpToDate(args: SectionUpgradeProps) {
    return <Component {...args} status={{ kind: 'up-to-date', upgradeInfo: null }} />;
}

export function UpgradeAvailable(args: SectionUpgradeProps) {
    return <Component {...args} status={{ kind: 'upgrade-available', upgradeInfo }} />;
}

export function PackagesAvailable(args: SectionUpgradeProps) {
    return <Component {...args} status={{ kind: 'upgrade-available', upgradeInfo: packageUpgradeInfo }} />;
}

export function UpgradingFirmware(args: SectionUpgradeProps) {
    return (
        <Component
            {...args}
            status={{
                kind: 'upgrading',
                upgradeInfo,
                progress: {
                    shape: 'firmware',
                    firmwarePhase: FirmwareUpgradePhase.DOWNLOADING,
                    packagePhase: null,
                    download: {
                        $typeName: 'braiins.bmc.web.UpgradeDownloadProgress',
                        downloadedBytes: 150_000_000n,
                        totalBytes: 365_000_000n,
                    },
                    startTime: gen.timestamp.minutes(-0.1),
                },
            }}
        />
    );
}

export function UpgradingPackages(args: SectionUpgradeProps) {
    return (
        <Component
            {...args}
            status={{
                kind: 'upgrading',
                upgradeInfo: packageUpgradeInfo,
                progress: {
                    shape: 'package',
                    firmwarePhase: null,
                    packagePhase: PackageUpgradePhase.BUILDING,
                    download: null,
                    startTime: gen.timestamp.minutes(-1.5),
                },
            }}
        />
    );
}

export function Restarting(args: SectionUpgradeProps) {
    return (
        <Component
            {...args}
            status={{
                kind: 'restarting',
                upgradeInfo: null,
                disruption: UpgradeDisruption.REBOOT,
                startTime: gen.timestamp.minutes(-0.23),
            }}
        />
    );
}
