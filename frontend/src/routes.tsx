import { createBrowserRouter } from 'react-router';

import { URLS } from './constants';
import { Root } from './app/Root';
import { Login } from './app/Login';
import { ChangePassword } from './app/ChangePassword';

export default createBrowserRouter([
    {
        path: '/',
        Component: Root,
        children: [
            {
                path: URLS.login,
                Component: Login,
            },
            {
                path: URLS.changePassword,
                Component: ChangePassword,
            },
        ],
    },
]);
