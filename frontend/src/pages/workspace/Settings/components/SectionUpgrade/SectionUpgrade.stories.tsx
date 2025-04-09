import type { Meta } from '@storybook/react';
import { action } from '@storybook/addon-actions';
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
        upgradeInfo: {
            $typeName: 'braiins.bmc.web.CheckForUpgradeResponse',
            latestRelease: {
                $typeName: 'braiins.bmc.web.UpgradeMetadata',
                version: '24.09.4',
                hash: 'db777c17acb949bcba3a69ba12875857',
                releaseDate: '2024-08-27',
                description:
                    'Introducing Braiins OS version 24.08! This update brings initial support for Antminer S19 XP Hydro and Antminer T21 Zynq/Xilinx control board, along with several enhancements. We have also improved the user experience to ensure smoother performance.\n\n## Antminer S21 & S19\n\n- Support of all BOS features now available for Antminer S19 XP Hydro\n- Support of all BOS features now available for Antminer T21 Zynq/Xilinx control board',
            },
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
                        'Introducing Braiins OS version 24.06, featuring significantly improved tuning time for 1366BM/1368BM and BeagleBones control boards, few features for Braiins Mini Miner, few bug fixes and more! \n\n## Antminer Family\n\n- Improved tuning time for 1366BM/1368BM chips\n- Significantly improved tuning time for BeagleBone control board\n- Miners are not restart-cycling anymore when the dangerous temperature is hit. Miner is put into persistent pause state. Resume command needs to be applied.\n- Better DNS caching has been implemented to reduce network bandwidth\n- Added troubleshooting for error codes \n- Added support for APW111721c PSU\n- Addressed a minor UX bugs and issue where users were unable to increase power target via API\n\n## Braiins Mini Miner\n\n- Users can now upgrade BOS via GUI \n- Users can now rotate screens with the button on the rear side of the BMM100\n- Addressed a bug where night mode sometimes did not turn the backlight on\n',
                },
            ],
        },
    } satisfies SectionUpgradeProps,
} satisfies Meta<SectionUpgradeProps>;

export function SectionUpgrade(args: SectionUpgradeProps) {
    return <Component {...args} />;
}
