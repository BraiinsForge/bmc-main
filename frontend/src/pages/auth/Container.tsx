import { Component } from 'react';
import { Outlet } from 'react-router';
import { LayoutPlain } from '@/layouts';

export default class AuthContainer extends Component {
    render() {
        return (
            <LayoutPlain>
                <Outlet />
            </LayoutPlain>
        );
    }
}
