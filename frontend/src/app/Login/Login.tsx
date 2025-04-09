import { Component } from 'react';
import * as pb from '@/proto';
import { store } from '@/store';

import { Form, type iFormErrors } from '@/lib/form';
import { TextInput, PasswordInput } from '@carbon/react';
import { Button, InlineNotificationsGroup } from '@/components';

import css from './Login.scss';

interface Data {
    username: string;
    password: string;
}

interface State {
    data: Data;
    errors: iFormErrors<keyof Data>;
}
const getInitialState = (): State => ({
    data: { username: '', password: '' },
    errors: {},
});

export class Login extends Component<any, State> {
    readonly state = getInitialState();

    componentWillUnmount() {
        pb.abort.all(this);
    }

    private abortLogin = pb.abort.get();
    #submit = async (): Promise<void> => {
        const { signal } = this.abortLogin.replace();
        const { data } = this.state;
        try {
            const response = await pb.rpc.auth.login(pb.create(pb.LoginRequestSchema, data), { signal });
            store.token = response.token;
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
        const title = 'Login';

        return (
            <div className={css.root}>
                <title children={title} />
                <dialog open className={css.modal}>
                    <header className={css.header} children={title} />
                    <Form className={css.form}>
                        <InlineNotificationsGroup items={errors.global} theme="inverse" kind="error" stretch />
                        <TextInput
                            id="login-username"
                            autoComplete="username"
                            labelText="Username"
                            value={data.username}
                            invalid={!!errors.fields?.username}
                            invalidText={errors.fields?.username}
                            onChange={e => this.#set('username', e.target.value)}
                        />
                        <PasswordInput
                            id="login-password"
                            labelText="Password"
                            autoComplete="current-password"
                            value={data.password}
                            invalid={!!errors.fields?.password}
                            invalidText={errors.fields?.password}
                            onChange={e => this.#set('password', e.target.value)}
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
