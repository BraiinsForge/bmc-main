import type { Meta } from '@storybook/react';
import { action } from '@storybook/addon-actions';
import { FormPasswordChange as Component, type FormPasswordChangeProps } from './FormPasswordChange';

type Args = Record<'passCurrent' | 'passNew' | 'passConfirm', string>;

export default {
    title: 'settings/components/FormPasswordChange',
    component: Component,
    args: {
        // Fields
        passCurrent: 'cbdb467d-157a-4167-adb6-520f52cae37c',
        passNew: '407335fd-b3d4-46d2-8d27-04e1cd798da9',
        passConfirm: '3d85e883-eb54-41e3-9e83-af846e6d3508',
    } satisfies Args,
} satisfies Meta<Args>;

export function FormPasswordChange(args: Args) {
    const { passCurrent, passNew, passConfirm } = args;

    const props: FormPasswordChangeProps = {
        passCurrent: {
            value: passCurrent,
            error: 'Our great beauty for futility is to need others wisely.',
            disabled: false,
            onChange: action('passCurrent.onChange'),
        },
        passNew: {
            value: passNew,
            error: 'Peritus, placidus speciess nunquam manifestum de noster, altus idoleum.',
            disabled: false,
            onChange: action('passNew.onChange'),
        },
        passConfirm: {
            value: passConfirm,
            error: 'The bliss is an ultimate believer!',
            disabled: false,
            onChange: action('passConfirm.onChange'),
        },
    };

    return (
        <div
            style={{
                display: 'inline-flex',
                flexDirection: 'column',
                gap: 8,
                width: 600,
                textAlign: 'center',
            }}
        >
            <h5 children="Change Password" />
            <div style={{ background: 'var(--cds-background)', padding: 16 }}>
                <Component {...props} />
            </div>

            <h5 children="Create Password" />
            <div style={{ background: 'var(--cds-background)', padding: 16 }}>
                <Component {...props} passCurrent={null} />
            </div>

            <h5 children="Remove Password" />
            <div style={{ background: 'var(--cds-background)', padding: 16 }}>
                <Component {...props} passNew={null} passConfirm={null} />
            </div>
        </div>
    );
}
