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

import { useIntl } from 'react-intl';

import { Button, LogoHeaderMiner, LogoHeader } from '@/components';
import { getID } from '@/lib/form';
import { Layout } from '../Layout';
import image from './hero-image.png';
import minerImage from './hero-image-miner.png';

// Styles
import css from './Welcome.scss';

export interface WelcomeProps {
    /// Product display name from the device capabilities ("Braiins Deck", "Mini Miner").
    productName?: string;
    /// Miner devices get their own hero image and the plain Braiins logo.
    miner?: boolean;
    onNext(): void;
}

const $ = getID('initial-setup-welcome').get;
export function Welcome(props: WelcomeProps) {
    const { onNext, productName, miner } = props;
    const { formatMessage } = useIntl();

    return (
        <Layout
            header={
                miner ? (
                    <LogoHeaderMiner style={{ width: 'auto', height: 18 }} />
                ) : (
                    <LogoHeader style={{ width: 'auto', height: 18 }} />
                )
            }
            footer={
                <Button
                    id={$('continue')}
                    kind="primary"
                    onClick={onNext}
                    children={formatMessage({
                        defaultMessage: 'Continue',
                        description: 'Button on the setup welcome page, opening the Wi-Fi step',
                    })}
                />
            }
        >
            <img src={miner ? minerImage : image} alt="" />
            <h1
                className={css.title}
                children={formatMessage(
                    {
                        defaultMessage: 'Welcome to Your New {product}!',
                        description:
                            'Title of the setup welcome page; product is the device name, such as Braiins Deck',
                    },
                    { product: productName || 'Braiins Device' },
                )}
            />
            <p
                className={css.text}
                children={formatMessage({
                    defaultMessage:
                        'Let’s get your device up and running. You’ll walk through a few quick setup steps to configure network and access settings.',
                    description: 'Introduction under the title of the setup welcome page',
                })}
            />
        </Layout>
    );
}
