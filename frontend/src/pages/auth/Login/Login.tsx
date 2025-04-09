import { Component, createRef } from 'react';
import { Helmet } from '@dr.pogodin/react-helmet';

import * as pb from '@/proto';
import { store } from '@/store';

import { Form, type iFormErrors } from '@/lib/form';
import { PasswordInput } from '@carbon/react';
import { Button, InlineNotificationsGroup } from '@/components';

import css from './Login.scss';

interface Data {
    password: string;
}

interface State {
    data: Data;
    errors: iFormErrors<keyof Data>;
}
const getInitialState = (): State => ({
    data: { password: '' },
    errors: {},
});

export default class LoginPage extends Component<any, State> {
    readonly state = getInitialState();
    #ref = createRef<HTMLDivElement>();

    componentDidMount() {
        this.#ref.current?.querySelector('input')?.select();
    }
    componentWillUnmount() {
        pb.abort.all(this);
    }

    private abortLogin = pb.abort.get();
    #submit = async (): Promise<void> => {
        const { signal } = this.abortLogin.replace();
        const { data } = this.state;
        try {
            await store.login(data.password, signal);
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({
                errors: pb.parseFormErrors(pb.parseError($), Object.keys(getInitialState().data)),
            });
            this.#ref.current?.querySelector('input')?.select();
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
            <div className={css.root} ref={this.#ref}>
                <Helmet title={title} />
                <dialog open className={css.modal}>
                    <header className={css.header} children={title} />
                    <Form className={css.form}>
                        <InlineNotificationsGroup items={errors.global} theme="inverse" kind="error" stretch />
                        <PasswordInput
                            id="login-password"
                            labelText="Password"
                            autoComplete="current-password"
                            value={data.password}
                            invalid={!!errors.fields?.password}
                            invalidText={errors.fields?.password}
                            onChange={e => this.#set('password', e.target.value)}
                        />
                        <footer className={css.footer}>
                            <Button type="submit" children={title} onClick={this.#submit} className={css.submit} />
                        </footer>
                    </Form>
                </dialog>
            </div>
        );
    }
}
