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

import { useRef, useEffect } from 'react';
import { useNavigate } from 'react-router';

import lottie from 'lottie-web';
import { useIntl } from 'react-intl';
import { Helmet } from '@/lib/react';
import { getID } from '@/lib/form';

import { LayoutPlain } from '../LayoutPlain';
import { Html, Button } from '@/components';
import css from './LayoutStatusPage.scss';

const $ = getID('status-page').get;

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
                        id={$('go-to-homepage')}
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
