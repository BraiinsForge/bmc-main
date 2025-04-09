import { Component } from 'react';
import { debounce } from 'es-toolkit';
import { Outlet, useNavigate, type NavigateFunction, useLocation } from 'react-router';

import { URLS } from '@/constants';
import { useStore, type AuthState } from '@/store';

import '@/styles/carbon/carbon.global.scss';

interface Props {
    pathname: string;
    navigate: NavigateFunction;
    isRootPath: boolean;
    isAuthenticated: AuthState;
}

class View extends Component<Props> {
    componentDidMount = () => this.#mount();
    componentDidUpdate(prevProps: Readonly<Props>) {
        const { isRootPath, isAuthenticated } = this.props;
        // Since there is nothing usefull on the roo path,
        // we have to always redirect it to "somthing"
        if (prevProps.isAuthenticated !== isAuthenticated || isRootPath) this.#maybeRedirect();
    }

    /**
     * Mount methods are extracted into their own debounced method to avoid problems arising from react's double render.
     * Instead of needless setup/teardown multiple times we'll avoid the problem by debouncing the mount method.
     * It does introduce a slight delay, but it's much more elegant and ultimately performant.
     */
    #mount = debounce(async () => {
        this.#maybeRedirect();
    }, 150);
    #maybeRedirect = async (): Promise<void> => {
        const { navigate, pathname, isAuthenticated, isRootPath } = this.props;
        const { login } = URLS.auth;

        const isPublicPage: boolean = Object.values(URLS.auth).some(x => pathname.startsWith(x));

        // Redirects based on authentication status
        //  - redirect to dashboard "from login / signup" if switched to authenticated
        //  - redirect to login if switched to UNauthenticated
        if (isAuthenticated === true && (isPublicPage || isRootPath)) return navigate(URLS.defaultScreen);
        if (isAuthenticated === false && !isPublicPage) return navigate(login);
    };

    render = () => <Outlet />;
}

export default function () {
    const { pathname } = useLocation();
    const navigate = useNavigate();
    const isRootPath: boolean = pathname === '/';
    const isAuthenticated = useStore(x => x.isAuthenticated);
    return <View navigate={navigate} pathname={pathname} isAuthenticated={isAuthenticated} isRootPath={isRootPath} />;
}
