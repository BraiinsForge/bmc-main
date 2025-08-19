import { RadioButton as RB, type RadioButtonProps as RBP } from '@carbon/react';

// Styles & testing
import css from './RadioButton.scss';
import cn from 'clsx';

type BaseProps = Omit<RBP, 'labelText'>;
export interface RadioButtonProps extends BaseProps {
    label?: NonNullable<ReactNode>;

    description?: ReactNode;
    descriptionClassName?: string;

    addon?: ReactNode;
    darker?: boolean;
    stretch?: boolean;
}

export function RadioButton(props: RadioButtonProps) {
    const {
        // Content
        label,
        description,
        descriptionClassName,
        addon,
        // Modifiers
        darker,
        stretch,
        // DOM
        className,
        style,
        ...rest
    } = props;

    let $label: ReactNode = label;
    if (label || description) {
        $label = (
            <span className={css.labelWrapper}>
                <span className={css.label} children={label} />
                {description && <span className={cn(css.description, descriptionClassName)} children={description} />}
            </span>
        );
    }

    if (addon) {
        $label = (
            <div className={css.addonWrapper}>
                {label}
                <div className={css.addon} children={addon} />
            </div>
        );
    }

    const cname = cn(css.root, darker && css.darker, stretch && css.stretch, className);
    return <RB {...rest} labelText={$label} className={cname} style={style} />;
}
