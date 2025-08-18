import { Component, createRef } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Helmet } from '@dr.pogodin/react-helmet';

import * as pb from '@/proto';
import { store } from '@/store';

import { Form } from '@/lib/form';
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

class View extends Component<Props, State> {
    readonly state = getInitialState();
    #ref = createRef<HTMLDivElement>();

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

                <div className={css.containerForm}>
                    <Form className={css.form}>
                        <LogoHeader width="auto" height={18} className={css.logo} />

                        <InlineNotificationsGroup items={errors?.global} theme="inverse" kind="error" stretch />

                        <PasswordInput
                            id="login-password"
                            labelText="Password"
                            autoComplete="current-password"
                            value={data.password}
                            invalid={!!errors?.fields?.password}
                            invalidText={pb.renderFieldErrorsAsList(errors?.fields?.password)}
                            onChange={e => this.#set('password', e.target.value)}
                        />

                        <Button
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
