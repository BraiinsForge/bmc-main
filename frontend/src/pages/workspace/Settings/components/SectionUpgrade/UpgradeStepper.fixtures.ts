import * as pb from '@/proto';
import * as gen from '@/mocks';
import type { UpgradeStepperProps } from './UpgradeStepper';

const startTime = gen.timestamp.minutes(-0.25);

const dl = (downloaded: number, total: number): pb.UpgradeDownloadProgress => ({
    $typeName: 'braiins.bmc.web.UpgradeDownloadProgress',
    downloadedBytes: BigInt(downloaded),
    totalBytes: BigInt(total),
});

// One snapshot per shape/step, shared by the story and the spec so they can't drift.
export const fixtures = {
    'firmware / downloading': {
        shape: 'firmware',
        firmwarePhase: pb.FirmwareUpgradePhase.DOWNLOADING,
        packagePhase: null,
        download: dl(6_000_000, 24_000_000),
        startTime,
    },
    'firmware / downloading (no total)': {
        shape: 'firmware',
        firmwarePhase: pb.FirmwareUpgradePhase.DOWNLOADING,
        packagePhase: null,
        download: dl(6_000_000, 0),
        startTime,
    },
    'firmware / verifying': {
        shape: 'firmware',
        firmwarePhase: pb.FirmwareUpgradePhase.VERIFYING,
        packagePhase: null,
        download: null,
        startTime,
    },
    'firmware / applying': {
        shape: 'firmware',
        firmwarePhase: pb.FirmwareUpgradePhase.APPLYING,
        packagePhase: null,
        download: null,
        startTime,
    },
    'package / realizing': {
        shape: 'package',
        firmwarePhase: null,
        packagePhase: pb.PackageUpgradePhase.REALIZING,
        download: dl(1_000_000, 4_000_000),
        startTime,
    },
    'package / verifying': {
        shape: 'package',
        firmwarePhase: null,
        packagePhase: pb.PackageUpgradePhase.VERIFYING,
        download: null,
        startTime,
    },
    'package / building': {
        shape: 'package',
        firmwarePhase: null,
        packagePhase: pb.PackageUpgradePhase.BUILDING,
        download: null,
        startTime,
    },
    'package / activating': {
        shape: 'package',
        firmwarePhase: null,
        packagePhase: pb.PackageUpgradePhase.ACTIVATING,
        download: null,
        startTime,
    },
    'package / finalizing': {
        shape: 'package',
        firmwarePhase: null,
        packagePhase: pb.PackageUpgradePhase.ACTIVATING,
        download: null,
        startTime,
        finalizing: true,
    },
    'combined / fw-download': {
        shape: 'combined',
        firmwarePhase: pb.FirmwareUpgradePhase.DOWNLOADING,
        packagePhase: null,
        download: dl(6_000_000, 24_000_000),
        startTime,
    },
    'combined / fw-verify': {
        shape: 'combined',
        firmwarePhase: pb.FirmwareUpgradePhase.VERIFYING,
        packagePhase: null,
        download: null,
        startTime,
    },
    'combined / pkg-realize': {
        shape: 'combined',
        firmwarePhase: pb.FirmwareUpgradePhase.VERIFYING,
        packagePhase: pb.PackageUpgradePhase.REALIZING,
        download: dl(1_000_000, 4_000_000),
        startTime,
    },
    'combined / pkg-verify': {
        shape: 'combined',
        firmwarePhase: pb.FirmwareUpgradePhase.VERIFYING,
        packagePhase: pb.PackageUpgradePhase.VERIFYING,
        download: null,
        startTime,
    },
    'combined / pkg-build': {
        shape: 'combined',
        firmwarePhase: pb.FirmwareUpgradePhase.VERIFYING,
        packagePhase: pb.PackageUpgradePhase.BUILDING,
        download: null,
        startTime,
    },
    'combined / fw-apply': {
        shape: 'combined',
        firmwarePhase: pb.FirmwareUpgradePhase.APPLYING,
        packagePhase: pb.PackageUpgradePhase.BUILDING,
        download: null,
        startTime,
    },
} satisfies Record<string, UpgradeStepperProps>;
