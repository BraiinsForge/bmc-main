import type { ImgHTMLAttributes } from 'react';
import { assertUnreachable } from '@/lib/ts';
import * as pb from '@/proto';

export interface ClockScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: pb.SceneVariantClock;
}
export function ClockScenePreview(props: ClockScenePreviewProps) {
    const { kind, ...rest } = props;

    switch (kind) {
        case pb.SceneVariantClock.analog_rect:
            return <img {...rest} src={require('./clock-analog-rect.png')} alt="Analog rectangular" />;

        case pb.SceneVariantClock.analog_round:
            return <img {...rest} src={require('./clock-analog-round.png')} alt="Analog round" />;

        case pb.SceneVariantClock.digital_flip:
            return <img {...rest} src={require('./clock-digital-flip.png')} alt="Digital flip" />;

        case pb.SceneVariantClock.digital_plain:
            return <img {...rest} src={require('./clock-digital-plain.png')} alt="Digital plain" />;

        default:
            assertUnreachable(kind, 'clock scene preview');
    }
}
