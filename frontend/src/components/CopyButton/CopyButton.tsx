import type { ComponentProps } from 'react';
import copy2clipboard from 'copy-to-clipboard';

import cn from 'clsx';
import css from './CopyButton.scss';

import { CopyButton as BaseComponent } from '@carbon/react';

type BaseProps = Omit<ComponentProps<typeof BaseComponent>, keyof LocalProps | 'tooltipAlignment' | 'tooltipPosition'>;
type LocalProps = {
    value: Maybe<string>;
    kind?: null | 'transparent' | 'light' | 'input-addon';
};

export type CopyButtonProps = BaseProps & LocalProps;

export function CopyButton(props: CopyButtonProps) {
    const { value, kind, className, ...rest } = props;
    const disabled = value == null || value === '';

    const handleClick = () => {
        if (value) copy2clipboard(value);
    };

    const p = {
        ...rest,
        onClick: handleClick,
        disabled,
        style: { cursor: disabled ? 'not-allowed' : 'pointer', ...(rest.style || {}) },
        className: cn(css.root, kind === 'light' && css.light, kind === 'input-addon' && css.inputAddon, className),
    };
    if (disabled) p.title = undefined;
    if (kind === 'transparent') p.style.backgroundColor = 'transparent';

    return <BaseComponent {...p} />;
}
