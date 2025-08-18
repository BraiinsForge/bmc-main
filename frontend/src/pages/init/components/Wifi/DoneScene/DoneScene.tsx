import { useIntl } from 'react-intl';

import { Layout } from '../../Layout';
import Image from './done-image.svg';
import { LogoHeader } from '@/components';

import css from './DoneScene.scss';

export function DoneScene() {
    const { formatMessage } = useIntl();

    return (
        <Layout header={<LogoHeader width="auto" height={18} style={{ width: 'auto' }} />}>
            <div className={css.root}>
                <Image width={100} />
                <h1
                    className={css.title}
                    children={formatMessage({
                        defaultMessage: 'Please follow up instructions on the BMC screen to continue.',
                    })}
                />
            </div>
        </Layout>
    );
}
