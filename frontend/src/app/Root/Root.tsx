import { Outlet, useNavigate } from 'react-router';

import { Button } from '@/components';
import { URLS } from '@/constants';

import '@/styles/carbon/carbon.global.scss';
import css from './Root.scss';

export function Root() {
    const navigate = useNavigate();
    return (
        <div className={css.root}>
            <header className={css.header}>
                <Button kind="tertiary" size="sm" children="Login" onClick={() => navigate(URLS.login)} />
                <Button
                    kind="tertiary"
                    size="sm"
                    children="Change Password"
                    onClick={() => navigate(URLS.changePassword)}
                />
            </header>

            <main className={css.main}>
                <Outlet />
            </main>

            <footer className={css.footer}>Footer</footer>
        </div>
    );
}
