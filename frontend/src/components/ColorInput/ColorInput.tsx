import css from './ColorInput.scss';
import cn from 'clsx';

export interface RGB {
    r: number;
    g: number;
    b: number;
}
function hexToRgb(hex: string): RGB {
    const n = Number.parseInt(hex.slice(1), 16);
    return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

export interface ColorInputProps {
    id: string;
    labelText?: string;
    'aria-label'?: string;

    value: string;
    onChange?(value: string, rgb: RGB): void;
    disabled?: boolean;

    className?: string;
    style?: CSSProperties;
}

export function ColorInput(props: ColorInputProps) {
    const { id, value, onChange, labelText, disabled, style, className } = props;
    const ariaLabel = props['aria-label'];

    return (
        <div className={cn(css.root, className)} style={style}>
            {labelText && (
                <label className="cds--label" htmlFor={id}>
                    {labelText}
                </label>
            )}
            <input
                type="color"
                id={id}
                className={css.input}
                value={value || '#000000'}
                onChange={e => onChange?.(e.target.value, hexToRgb(e.target.value))}
                aria-label={ariaLabel}
                disabled={disabled}
            />
        </div>
    );
}
