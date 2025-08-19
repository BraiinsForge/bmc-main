import cn from 'clsx';
import { Checkbox } from '@/components/Checkbox';
import { RadioButton } from '@/components/RadioButton';

export interface TableSelectRowProps {
    // ids
    id: string;
    name: string;
    ariaLabel: string;

    // state
    checked: boolean;
    disabled?: boolean;
    radio: boolean;

    // callbacks
    onChange(checked: boolean): void;
    render?(children: ReactElement): ReactElement;

    // Visual
    className?: string;
    style?: CSSProperties;
}

export function TableSelectRow(props: TableSelectRowProps) {
    const {
        // ids
        id,
        name,
        ariaLabel,
        // state
        radio,
        checked,
        disabled,
        // callbacks
        onChange,
        render = x => x,
        // visual
        className,
        style,
    } = props;

    return (
        <td
            style={style}
            className={cn('cds--table-column-checkbox', radio && 'cds--table-column-radio', className)}
            children={render(
                radio ? (
                    <RadioButton
                        id={id}
                        name={name}
                        onChange={(_, __, e) => onChange((e.target satisfies HTMLInputElement).checked)}
                        checked={checked}
                        disabled={disabled}
                        label={ariaLabel}
                        hideLabel
                    />
                ) : (
                    <Checkbox
                        id={id}
                        name={name}
                        onChange={e => onChange(e.target.checked)}
                        checked={checked}
                        disabled={disabled}
                    />
                ),
            )}
        />
    );
}
