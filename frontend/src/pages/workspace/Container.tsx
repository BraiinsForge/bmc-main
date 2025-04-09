import { Component } from 'react';
import { Outlet } from 'react-router';
import { LayoutWorkspace } from '@/layouts';

export default class WorkspaceContainer extends Component {
    render() {
        return (
            <LayoutWorkspace>
                <Outlet />
            </LayoutWorkspace>
        );
    }
}
