import { Component } from 'react';
import * as pb from '@/proto';
import { Form, type iFormErrors } from '@/lib/form';

import { TextInput, PasswordInput } from '@carbon/react';
import { Button, InlineNotificationsGroup } from '@/components';
import css from './ChangePassword.scss';

interface Data {
    old: string;
    new: string;
}

interface State {
    data: Data;
    errors: iFormErrors<keyof Data>;
}
const getInitialState = (): State => ({
    data: { old: '', new: '' },
    errors: {},
});

export class ChangePassword extends Component<any, State> {
    readonly state = getInitialState();

    componentWillUnmount() {
        pb.abort.all(this);
    }

    private abortLogin = pb.abort.get();
    #submit = async (): Promise<void> => {
        const { signal } = this.abortLogin.replace();
        const { data } = this.state;
        try {
            await pb.rpc.sys.setPassword(
                pb.create(pb.SetPasswordRequestSchema, {
                    currentPassword: data.old,
                    newPassword: data.new,
                }),
                { signal },
            );
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({
                errors: pb.parseFormErrors(pb.parseError($), Object.keys(getInitialState().data)),
            });
        }
    };
    #set = <Key extends keyof Data>(key: Key, value: Data[Key]): void => {
        this.setState(s => ({
            data: { ...s.data, [key]: value },
            errors: {},
        }));
    };

    render() {
        const { data, errors } = this.state;
        const title = 'Change Password';

        return (
            <div className={css.root}>
                <title children={title} />
                <dialog open className={css.modal}>
                    <header className={css.header} children={title} />
                    <Form className={css.form}>
                        <InlineNotificationsGroup items={errors.global} theme="inverse" kind="error" stretch />
                        <TextInput
                            id="login-username"
                            labelText="Current Passowrd"
                            autoComplete="current-password"
                            value={data.old}
                            invalid={!!errors.fields?.old}
                            invalidText={errors.fields?.old}
                            onChange={e => this.#set('old', e.target.value)}
                        />
                        <PasswordInput
                            id="login-password"
                            autoComplete="new-password"
                            labelText="New Password"
                            value={data.new}
                            invalid={!!errors.fields?.new}
                            invalidText={errors.fields?.new}
                            onChange={e => this.#set('new', e.target.value)}
                        />
                    </Form>
                    <footer className={css.footer}>
                        <Button children={title} onClick={this.#submit} className={css.submit} />
                    </footer>
                </dialog>
            </div>
        );
    }
}
