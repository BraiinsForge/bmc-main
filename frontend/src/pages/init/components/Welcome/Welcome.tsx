import { Button, CombinedLogo } from '@/components';
import { Layout } from '../Layout';
import image from './hero-image.png';

// Styles
import css from './Welcome.scss';

export interface WelcomeProps {
    onNext(): void;
}

export function Welcome(props: WelcomeProps) {
    const { onNext } = props;

    return (
        <Layout header={<CombinedLogo />} footer={<Button kind="primary" onClick={onNext} children="Continue" />}>
            <img src={image} alt="hero" />
            <h1 className={css.title} children="Welcome to Your New BMC!" />
            <p className={css.text}>
                Let’s get your clocks up and running. You’ll walk through a few quick setup steps to configure time,
                network, and access settings.
            </p>
        </Layout>
    );
}
