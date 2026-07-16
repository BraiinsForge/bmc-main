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

import type { HTMLAttributes } from 'react';
import { useIntl } from 'react-intl';

// Components
import { Tooltip } from '@/components';
import { type CarbonIconType, Asleep as IconNight } from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import css from './SceneTypeIcon.scss';

export interface SceneTypeIconsProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
    night?: boolean;
}

interface PillProps {
    icon: CarbonIconType;
    text: string;
    className: string;
}
function Pill(props: PillProps) {
    const { icon: Icon, text, className } = props;
    return (
        <Tooltip
            placement="bottom"
            content={text}
            render={r => (
                <div className={cn(css.pill, className)} ref={r}>
                    <Icon size={16} />
                </div>
            )}
        />
    );
}

export function SceneTypeIcons(props: SceneTypeIconsProps) {
    const { night, className, ...rest } = props;
    const { formatMessage } = useIntl();

    const content: ReactNode[] = [];
    if (night) {
        content.push(
            <Pill
                key="night"
                icon={IconNight}
                className={css.night}
                text={formatMessage({
                    defaultMessage:
                        'Night Mode - First widget stays displayed during the night mode, when the rotation is disabled.',
                })}
            />,
        );
    }

    return <div {...rest} className={cn(className, css.root)} children={content} />;
}
