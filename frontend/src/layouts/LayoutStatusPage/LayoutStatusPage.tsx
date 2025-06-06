import { useRef, useEffect } from 'react';
import { useNavigate } from 'react-router';

import lottie from 'lottie-web';
import { useIntl } from 'react-intl';
import { Helmet } from '@/lib/react';

import { LayoutPlain } from '../LayoutPlain';
import { Html, Button } from '@/components';
import css from './LayoutStatusPage.scss';

export interface StatusPageProps {
    homepageButton?: boolean;
    lottieData: Record<string, any>;
    title: string;
    h1: string;
    h2?: string;
}
export function LayoutStatusPage(props: StatusPageProps) {
    const { homepageButton, lottieData, title, h1, h2 } = props;
    const { formatMessage } = useIntl();
    const navigate = useNavigate();

    const ref = useRef<HTMLDivElement>(null);
    const element = ref.current;

    useEffect(() => {
        if (!element) return;
        element.innerHTML = '';
        lottie.setQuality(2);
        lottie.loadAnimation({
            container: element,
            renderer: 'svg',
            loop: true,
            autoplay: true,
            animationData: lottieData,
        });
    }, [lottieData, element]);

    return (
        <LayoutPlain>
            <div className={css.layout}>
                <div ref={ref} className={css.animation} id="lottie" />
                <Html container="div" className={css.h1} source={h1} />
                {h2 && <Html container="div" className={css.h2} source={h2} />}

                {homepageButton ? (
                    <Button
                        kind="primary"
                        className={css.button}
                        onClick={() => navigate('/')}
                        children={formatMessage({ defaultMessage: 'Go to Homepage' })}
                    />
                ) : null}

                <Helmet title={title} />
            </div>
        </LayoutPlain>
    );
}
