import { createElement, type ComponentProps } from 'react';

// Components
import { Button } from '../Button';
import { ButtonGroup } from '../ButtonGroup';
import type { CarbonIconType } from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import css from './ButtonSwitch.scss';

type ButtonProps = ComponentProps<typeof Button>;

export interface ButtonSwitchItem<K = string, V extends StrNum = StrNum> {
    id: K;
    text?: V;
    icon?: CarbonIconType;
    title?: string;
    disabled?: boolean;
    readonly?: boolean;
}
export interface ButtonSwitchProps<K = string, V extends StrNum = StrNum> {
    id?: string;

    // Data
    options: ReadonlyArray<ButtonSwitchItem<K, V>>;
    selectedOption: undefined | null | K;

    // Visuals
    size?: ButtonProps['size'];
    stretch?: boolean;
    vertical?: boolean;
    textAlign?: 'start' | 'center' | 'end';
    fullSizeDelimiter?: boolean;

    // Labeling
    labelText?: ReactNode;
    helperText?: ReactNode;

    // Actionable
    disabled?: boolean;
    readonly?: boolean;

    // Errors
    invalid?: boolean;
    invalidText?: null | string | ReactElement;

    // Handlers
    onChange?(key: K): void;
    onClick?(key: K): void;

    // DOM
    className?: string;
    style?: CSSProperties;
}
export function ButtonSwitch<K = string, V extends StrNum = StrNum>(props: ButtonSwitchProps<K, V>) {
    const {
        // Data
        options,
        selectedOption,

        // Visuals
        size,
        stretch,
        vertical,
        textAlign,
        fullSizeDelimiter,

        // Actionable
        disabled,
        readonly,

        // Errors
        invalid,
        invalidText,

        // Handlers
        onChange,
        onClick,

        // Labeling
        labelText,
        helperText,

        // Pass-on
        className,
        ...rest
    } = props;
    if (!Array.isArray(options) || !options.length) return null;

    const buttons = options.map((btn, i) => {
        let $icon: ReactNode = null;
        if (btn.icon) $icon = createElement(btn.icon, { size: 20 });

        let handleClick: undefined | Fn;
        const isActionable: boolean = !(disabled || readonly || btn.disabled || btn.readonly);
        if (isActionable) {
            handleClick = () => {
                onClick?.(btn.id);
                if (onChange && selectedOption !== btn.id) onChange(btn.id);
            };
        }

        return (
            <Button
                key={i}
                id={btn.id}
                kind="secondary"
                size={size}
                disabled={disabled === true || btn.disabled === true || (!onClick && !onChange)}
                className={cn(css.button, btn.id === selectedOption && css.selected)}
                title={btn.title}
                data-selected={btn.id === selectedOption}
                onClick={handleClick}
            >
                <div className={css.buttonContent}>
                    {$icon}
                    {/**
                     * Since we're rendering the icon inside the text instead of the standard way of button,
                     * we need to wrap the text in it's own node as well so that flex centering works-out.
                     */}
                    {btn.text != null && <div children={btn.text} />}
                </div>
            </Button>
        );
    });
    const classNames = cn(
        css.root,
        invalid && css.invalid,
        vertical && css.vertical,
        stretch && css.stretch,
        readonly && css.readonly,
        fullSizeDelimiter && css.fullSizeDelimiter,
        textAlign && css[`text_${textAlign}`],
        // Pass-on
        className,
    );

    return (
        <div {...rest} className={classNames}>
            {!!labelText && <div className="cds--label" style={{ display: 'block' }} children={labelText} />}
            <ButtonGroup children={buttons} vertical={vertical} className={css.group} />
            {(!!helperText || !!invalidText) && (
                <div className="cds--form__helper-text" children={helperText || invalidText} />
            )}
        </div>
    );
}
