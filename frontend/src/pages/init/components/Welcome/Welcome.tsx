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

import { Button, LogoHeader } from '@/components';
import { getID } from '@/lib/form';
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
