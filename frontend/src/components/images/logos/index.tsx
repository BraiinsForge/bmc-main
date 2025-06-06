import { carbonizeSvgIcon } from '@/lib/react';
import css from './index.scss';

export const Braiins = carbonizeSvgIcon(require('./braiins.svg'), 'Braiins');
export const BMC = carbonizeSvgIcon(require('./bmc.svg'), 'BMC');

export function CombinedLogo() {
    return (
        <div className={css.logos}>
            <Braiins width={80} />
            <BMC width={77} />
        </div>
    );
}
