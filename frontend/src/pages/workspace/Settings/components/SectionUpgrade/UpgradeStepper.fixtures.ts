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
