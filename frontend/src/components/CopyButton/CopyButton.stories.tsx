import { TextInput } from '@carbon/react';
import { CopyButton as Component, type CopyButtonProps } from './CopyButton';

const kinds: Array<CopyButtonProps['kind']> = [null, 'light', 'transparent', 'input-addon'];

export default {
    title: 'components/CopyButton',
    component: Component,
    args: {
        value: '12345',
        align: 'bottom',
        disabled: false,
        kind: 'transparent',
    },
    argTypes: {
        align: {
            control: { type: 'select', options: ['left', 'right', 'bottom', 'top'] },
        },
        kind: {
            control: { type: 'select', options: kinds },
        },
        disabled: {
            control: { type: 'boolean' },
        },
    },
};

const WithInput = (props: CopyButtonProps) => {
    return (
        <div style={{ position: 'relative', display: 'flex', flexDirection: 'row', width: 200 }}>
            <TextInput id={String(props.value)} value={props.value ?? undefined} labelText="" />
            <Component {...props} />
        </div>
    );
};

function getBlock(comment: string, args: CopyButtonProps, withInput?: boolean) {
    const C = withInput ? WithInput : Component;

    return (
        <div
            key={comment}
            style={{
                display: 'flex',
                flexDirection: 'column',
                gap: '1rem',
                margin: 40,
            }}
        >
            <h1 children={comment} />
            {kinds.map(kind => {
                return (
                    <div key={kind}>
                        <p children={`{ kind: ${kind} }`} />
                        <C kind={kind} title="Button Title" {...args} />
                    </div>
                );
            })}
        </div>
    );
}

export function CopyButton(args: CopyButtonProps) {
    return [getBlock('Default', args, false), getBlock('With input', args, true)];
}
