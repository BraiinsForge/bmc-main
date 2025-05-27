import { format } from 'd3-format';

export interface PercentageProps {
    value?: null | number; // 0..1 | 0..100

    /**
     * Log10 multiplier for the value defaulting to 0.
     *
     * @example
     * `<Luck value={500} base={-2} precision={2} />` == `500 * 1e-2` == `50%`
     */
    base?: number;

    /**
     * Value less or equal to this will be treated
     * as nil and placeholder will be shown
     */
    lowerValueBound?: number;
    /**
     * Decides the domain in which the input value is treated…
     * that is float "from 0 to 1" or "from 1 to 100"
     */
    upperValueBound?: 1 | 100;

    round?: boolean | number;
    trim?: boolean; // Remove trailing zeros

    placeholder?: string;

    // Visuals
    className?: string;
    style?: CSSProperties;
}
export function Percentage(props: PercentageProps) {
    const {
        value,
        base,
        lowerValueBound = 0.0001,
        upperValueBound = 1,
        round,
        trim,
        placeholder = '< 0.01',
        ...rest
        //
    } = props;

    let formated = placeholder;
    if (value != null && Number.isFinite(value)) {
        let normalized: number = value;
        if (base && Number.isFinite(base)) normalized = value * 10 ** base;
        if (upperValueBound === 1) normalized *= 100;

        if (normalized > lowerValueBound) {
            const frac = typeof round === 'number' ? round : round ? 0 : 2;
            const fmt = trim
                ? // Fixed decimals, no trailing zeroes
                  format(`.${frac}~f`)
                : // Fixed number of decimals
                  format(`.${frac}f`);
            formated = fmt(normalized);
        }
    }

    return (
        <span dir="ltr" {...rest}>
            <data data-role="value" children={formated} />
            {/*
             English style guides prescribe writing the percent sign
             following the number without any space between (e.g. 50%).
            */}
            <span role="presentation" data-role="unit" children="%" />
        </span>
    );
}
