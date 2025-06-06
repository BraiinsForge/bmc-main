import l404 from './404.lottie.json';
import { useIntl } from 'react-intl';
import { LayoutStatusPage } from '../../layouts';

export function NotFound() {
    const { formatMessage } = useIntl();
    return (
        <LayoutStatusPage
            homepageButton
            lottieData={l404}
            title={formatMessage({ defaultMessage: 'Page Not Found' })}
            h1={formatMessage({ defaultMessage: "We couldn't find the page or\xa0file you're looking for." })}
        />
    );
}
