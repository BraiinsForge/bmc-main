import { defineMessages, useIntl, type MessageDescriptor } from 'react-intl';
import { CheckmarkOutline, Incomplete, CircleDash } from '@carbon/react/icons';

import * as pb from '@/proto';
import { Progressbar, Percentage, Tick, InlineLoading } from '@/components';
import C from '@/styles/colors';
import { formatBytes } from '@/lib/format';
import { assertUnreachable } from '@/lib/ts';
import css from './UpgradeStepper.scss';

export type UpgradeShape = 'firmware' | 'package' | 'combined';

// Snapshot of an in-progress upgrade: the latest phase of each track, kept as
// proto enums so a dropped/added phase breaks the exhaustive tables below.
export interface UpgradeStepperProps {
    shape: UpgradeShape;
    firmwarePhase: null | pb.FirmwareUpgradePhase;
    packagePhase: null | pb.PackageUpgradePhase;
    download: null | pb.UpgradeDownloadProgress;
    startTime: number;
    // After `finished`: activation is done and we're waiting out the restart
    // reconcile. Mark every step done and show an indeterminate "finishing" row.
    finalizing?: boolean;
}

const labels = defineMessages({
    downloading: { defaultMessage: 'Downloading' },
    downloadingFirmware: { defaultMessage: 'Downloading firmware' },
    downloadingPackages: { defaultMessage: 'Downloading packages' },
    verifying: { defaultMessage: 'Verifying' },
    verifyingFirmware: { defaultMessage: 'Verifying firmware' },
    verifyingPackages: { defaultMessage: 'Verifying packages' },
    building: { defaultMessage: 'Building' },
    buildingPackages: { defaultMessage: 'Building packages' },
    activating: { defaultMessage: 'Activating' },
    applying: { defaultMessage: 'Applying' },
});

interface StepDef {
    id: string;
    label: MessageDescriptor;
    isDownload: boolean;
}

// Ordered steps per shape. A combined run interleaves both tracks and omits
// package ACTIVATING (it folds into the firmware reboot).
const STEPS: Record<UpgradeShape, StepDef[]> = {
    firmware: [
        { id: 'fw-download', label: labels.downloading, isDownload: true },
        { id: 'fw-verify', label: labels.verifying, isDownload: false },
        { id: 'fw-apply', label: labels.applying, isDownload: false },
    ],
    package: [
        { id: 'pkg-realize', label: labels.downloading, isDownload: true },
        { id: 'pkg-verify', label: labels.verifying, isDownload: false },
        { id: 'pkg-build', label: labels.building, isDownload: false },
        { id: 'pkg-activate', label: labels.activating, isDownload: false },
    ],
    combined: [
        { id: 'fw-download', label: labels.downloadingFirmware, isDownload: true },
        { id: 'fw-verify', label: labels.verifyingFirmware, isDownload: false },
        { id: 'pkg-realize', label: labels.downloadingPackages, isDownload: true },
        { id: 'pkg-verify', label: labels.verifyingPackages, isDownload: false },
        { id: 'pkg-build', label: labels.buildingPackages, isDownload: false },
        { id: 'fw-apply', label: labels.applying, isDownload: false },
    ],
};

// Exhaustive phase → step-id: a new/removed proto phase breaks the build here.
function firmwareStepId(phase: null | pb.FirmwareUpgradePhase): 'fw-download' | 'fw-verify' | 'fw-apply' {
    switch (phase) {
        case null:
        case pb.FirmwareUpgradePhase.UNSPECIFIED:
        case pb.FirmwareUpgradePhase.DOWNLOADING:
            return 'fw-download';
        case pb.FirmwareUpgradePhase.VERIFYING:
            return 'fw-verify';
        case pb.FirmwareUpgradePhase.APPLYING:
            return 'fw-apply';
        default:
            return assertUnreachable(phase, 'firmware upgrade phase');
    }
}
function packageStepId(
    phase: null | pb.PackageUpgradePhase,
): 'pkg-realize' | 'pkg-verify' | 'pkg-build' | 'pkg-activate' {
    switch (phase) {
        case null:
        case pb.PackageUpgradePhase.UNSPECIFIED:
        case pb.PackageUpgradePhase.REALIZING:
            return 'pkg-realize';
        case pb.PackageUpgradePhase.VERIFYING:
            return 'pkg-verify';
        case pb.PackageUpgradePhase.BUILDING:
            return 'pkg-build';
        case pb.PackageUpgradePhase.ACTIVATING:
            return 'pkg-activate';
        default:
            return assertUnreachable(phase, 'package upgrade phase');
    }
}

// In a combined run packages sit between firmware verify and apply: APPLYING
// wins, then a live package phase (ACTIVATING → apply), then fw download/verify.
function currentStepId(props: UpgradeStepperProps): string {
    const { shape, firmwarePhase: fw, packagePhase: pkg } = props;
    switch (shape) {
        case 'firmware':
            return firmwareStepId(fw);
        case 'package':
            return packageStepId(pkg);
        case 'combined': {
            if (fw === pb.FirmwareUpgradePhase.APPLYING) return 'fw-apply';
            if (pkg != null && pkg !== pb.PackageUpgradePhase.UNSPECIFIED) {
                const id = packageStepId(pkg);
                return id === 'pkg-activate' ? 'fw-apply' : id;
            }
            return firmwareStepId(fw);
        }
        default:
            return assertUnreachable(shape, 'upgrade shape');
    }
}

type StepState = 'done' | 'active' | 'pending';
const STEP_ICON = { done: CheckmarkOutline, active: Incomplete, pending: CircleDash };

export function UpgradeStepper(props: UpgradeStepperProps) {
    const { formatMessage } = useIntl();
    const { shape, download, startTime, finalizing = false } = props;

    const steps = STEPS[shape];
    // Finalizing puts the cursor past the last step, so every step reads as done.
    const currentIndex = finalizing
        ? steps.length
        : Math.max(
              0,
              steps.findIndex(s => s.id === currentStepId(props)),
          );

    return (
        <>
            <ol className={css.steps}>
                {steps.map((s, i) => {
                    const state: StepState = i < currentIndex ? 'done' : i === currentIndex ? 'active' : 'pending';
                    const Icon = STEP_ICON[state];
                    return (
                        <li key={s.id} className={css.step}>
                            <div className={css.rail}>
                                <Icon
                                    size={16}
                                    className={css.marker}
                                    fill={state === 'pending' ? 'currentColor' : C.accentViolet}
                                />
                            </div>
                            <div className={css.body}>
                                <span children={formatMessage(s.label)} />
                                {state === 'active' ? (
                                    <div className={css.detail}>
                                        <ActiveDetail step={s} download={download} startTime={startTime} />
                                    </div>
                                ) : null}
                            </div>
                        </li>
                    );
                })}
            </ol>
            {finalizing ? (
                <InlineLoading
                    className={css.finishing}
                    status="active"
                    iconDescription="Finishing up"
                    description={formatMessage({ defaultMessage: 'Finishing up…' })}
                />
            ) : null}
        </>
    );
}

// The active step's live progress, inline: a byte bar when a download total is
// known, otherwise the bytes downloaded or the elapsed time.
function ActiveDetail({
    step,
    download,
    startTime,
}: {
    step: StepDef;
    download: null | pb.UpgradeDownloadProgress;
    startTime: number;
}) {
    const { formatMessage } = useIntl();
    const bytes = step.isDownload ? download : null;
    const total = bytes?.totalBytes != null ? Number(bytes.totalBytes) : null;

    if (bytes && total != null && total > 0) {
        const downloaded = Number(bytes.downloadedBytes);
        const percentage = (downloaded / total) * 100;
        return (
            <>
                <Progressbar
                    label={<Percentage value={percentage} upperValueBound={100} round={0} />}
                    labelPosition="top-left"
                    valueUpperBound={100}
                    values={[{ value: percentage, color: C.accentViolet, animate: true }]}
                />
                <div className={css.sub} children={`${formatBytes(downloaded)} / ${formatBytes(total)}`} />
            </>
        );
    }

    return (
        <Tick
            intervalMs={1e3}
            render={() => {
                const elapsed = Math.max(0, Math.floor(Date.now() / 1e3) - startTime);
                const text = bytes
                    ? formatMessage(
                          { defaultMessage: '{done} downloaded' },
                          { done: formatBytes(Number(bytes.downloadedBytes)) },
                      )
                    : formatMessage({ defaultMessage: '{elapsed}s elapsed' }, { elapsed });
                return <span className={css.sub} children={text} />;
            }}
        />
    );
}
