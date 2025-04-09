import type { ImgHTMLAttributes } from 'react';

import AnalaogRect from './clock-analog-rect.png';
import AnalogRound from './clock-analog-round.png';
import DigitalFlip from './clock-digital-flip.png';
import DigitalPlain from './clock-digital-plain.png';

export interface ClockScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    variant: 'analog-rect' | 'analog-round' | 'digital-flip' | 'digital-plain';
}
export function ClockScenePreview(props: ClockScenePreviewProps) {
    const { variant, ...rest } = props;

    switch (variant) {
        case 'analog-rect':
            return <img {...rest} src={AnalaogRect} alt="Analog rectangular" />;

        case 'analog-round':
            return <img {...rest} src={AnalogRound} alt="Analog round" />;

        case 'digital-flip':
            return <img {...rest} src={DigitalFlip} alt="Digital flip" />;

        case 'digital-plain':
            return <img {...rest} src={DigitalPlain} alt="Digital plain" />;
    }
}
