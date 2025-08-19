import css from './SoundPlayIcon.scss';
import { Loading } from '@carbon/react';
import { Play as IconPlay, Stop as IconStop } from '@carbon/react/icons';

export interface SoundPlayIconProps {
    isPlaying: boolean;
}
export function SoundPlayIcon(props: SoundPlayIconProps) {
    const { isPlaying } = props;

    return isPlaying ? (
        <div className={css.composedLoading}>
            <Loading small active withOverlay={false} className={css.spinner} />
            <IconStop className={css.icon} />
        </div>
    ) : (
        <IconPlay />
    );
}
