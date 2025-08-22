import { Button, LogoHeader } from '@/components';
import { getID } from '@/lib/form.tsx';
import { Layout } from '../Layout';
import image from './hero-image.png';

// Styles
import css from './Welcome.scss';

export interface WelcomeProps {
    onNext(): void;
}

const $ = getID('initial-setup-welcome').get;
export function Welcome(props: WelcomeProps) {
    const { onNext } = props;

    return (
        <Layout
            header={<LogoHeader style={{ width: 'auto', height: 18 }} />}
            footer={<Button id={$('continue')} kind="primary" onClick={onNext} children="Continue" />}
        >
            <img src={image} alt="hero" />
            <h1 className={css.title} children="Welcome to Your New Braiins DECK!" />
            <p className={css.text}>
                Let’s get your clocks up and running. You’ll walk through a few quick setup steps to configure time,
                network, and access settings.
            </p>
        </Layout>
    );
}
