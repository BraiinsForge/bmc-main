// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { useIntl } from 'react-intl';

import css from './SoundPlayIcon.scss';
// biome-ignore lint/style/noRestrictedImports: Carbon's small plain ring fits around the stop icon; ours draws the logo inside
import { Loading } from '@carbon/react';
import { Play as IconPlay, Stop as IconStop } from '@carbon/react/icons';

export interface SoundPlayIconProps {
    isPlaying: boolean;
}

/** The content of a button previewing a sound, which names the button by what a press does. */
export function SoundPlayIcon(props: SoundPlayIconProps) {
    const { isPlaying } = props;
    const { formatMessage } = useIntl();

    return isPlaying ? (
        <>
            <div className={css.composedLoading}>
                <Loading small active withOverlay={false} className={css.spinner} aria-hidden />
                <IconStop className={css.icon} />
            </div>
            <span
                className="cds--visually-hidden"
                children={formatMessage({
                    defaultMessage: 'Stop the sound',
                    description: 'Screen-reader name of the button previewing an alarm sound, while the sound plays',
                })}
            />
        </>
    ) : (
        <>
            <IconPlay />
            <span
                className="cds--visually-hidden"
                children={formatMessage({
                    defaultMessage: 'Play the sound',
                    description: 'Screen-reader name of the button previewing an alarm sound',
                })}
            />
        </>
    );
}
