import type { ImgHTMLAttributes } from 'react';
import { assertUnreachable } from '@/lib/ts';
import * as pb from '@/proto';

export interface ClockScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: pb.ClockWidget_ClockStyle;
}
export function ClockScenePreview(props: ClockScenePreviewProps) {
    const { kind, ...rest } = props;
    if (!kind) return null;

    switch (kind) {
        case pb.ClockWidget_ClockStyle.ANALOG_RECT:
            return <img {...rest} src={require('./clock-analog-rect.png')} alt="Analog rectangular" />;

        case pb.ClockWidget_ClockStyle.ANALOG_ROUND:
            return <img {...rest} src={require('./clock-analog-round.png')} alt="Analog round" />;

        case pb.ClockWidget_ClockStyle.DIGITAL:
            return <img {...rest} src={require('./clock-digital-plain.png')} alt="Digital plain" />;

        // case pb.SceneVariantClock.digital_flip:
        //     return <img {...rest} src={require('./clock-digital-flip.png')} alt="Digital flip" />;

        default:
            assertUnreachable(kind, 'clock scene preview');
    }
}
