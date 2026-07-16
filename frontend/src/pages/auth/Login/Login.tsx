// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { Component, createRef } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Helmet } from '@dr.pogodin/react-helmet';

import * as pb from '@/proto';
import { store } from '@/store';

import { Form, getID } from '@/lib/form';
import { PasswordInput } from '@carbon/react';
import { ArrowRight } from '@carbon/react/icons';
import { Button, InlineNotificationsGroup, LogoHeader } from '@/components';

import css from './Login.scss';

type Data = {
    password: string;
};

interface Props {
    intl: IntlShape;
}

interface State {
    data: Data;
    errors: null | pb.FormErrors<Data>;
}
const getInitialState = (): State => ({
    data: { password: '' },
    errors: null,
});

const $ = getID('login').get;
class View extends Component<Props, State> {
    readonly state = getInitialState();
    #ref = createRef<HTMLDivElement>();

    #refPassword = createRef<HTMLInputElement>();
    #focusPassword = (): void => {
        this.#refPassword.current?.focus();
    };

    #txt = {
        login: this.props.intl.formatMessage({ defaultMessage: 'Login' }),
    };

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
            this.setState({ errors: pb.parseFormErrors<Data>($, ['password']) });
            this.#ref.current?.querySelector('input')?.select();
        }
    };
    #set = <Key extends keyof Data>(key: Key, value: Data[Key]): void => {
        this.setState(s => ({
            data: { ...s.data, [key]: value },
            errors: null,
        }));
    };

    render() {
        const { data, errors } = this.state;

        return (
            <div className={css.root} ref={this.#ref}>
                <Helmet title={this.#txt.login} />

                {/** biome-ignore lint/a11y/useKeyWithClickEvents: Irrelevant for keyboard navigation. */}
                <div className={css.containerForm} onClick={this.#focusPassword}>
                    <Form className={css.form}>
                        <LogoHeader style={{ width: 'auto', height: 18 }} className={css.logo} />

                        <InlineNotificationsGroup items={errors?.global} theme="inverse" kind="error" stretch />

                        <PasswordInput
                            id="login-password"
                            ref={this.#refPassword}
                            labelText="Password"
                            autoComplete="current-password"
                            value={data.password}
                            invalid={!!errors?.fields?.password}
                            invalidText={pb.renderFieldErrorsAsList(errors?.fields?.password)}
                            onChange={e => this.#set('password', e.target.value)}
                        />

                        <Button
                            id={$('submit')}
                            type="submit"
                            children={this.#txt.login}
                            onClick={this.#submit}
                            className={css.submit}
                            icon={ArrowRight}
                        />
                    </Form>
                </div>

                <section className={css.containerImage} aria-hidden />
            </div>
        );
    }
}

export default function LoginPage() {
    const intl = useIntl();
    return <View intl={intl} />;
}
