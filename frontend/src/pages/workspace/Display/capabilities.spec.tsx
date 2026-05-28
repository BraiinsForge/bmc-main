import { afterEach, describe, test, expect } from '@rstest/core';
import { cleanup, fireEvent, render } from '@testing-library/react/pure';
import { MemoryRouter, Route, Routes } from 'react-router';
import { combinedEditorRedirectTarget, combinedSceneAvailable } from './fn';
import { CombinedSceneMenuAction } from './DisplayList';
import { CombinedEditorCapabilityGate } from './DisplayCombined';
import { URLS } from '@/constants';
import type * as pb from '@/proto';

afterEach(cleanup);

describe('combinedSceneAvailable', () => {
    test('true when backend reports combined scenes supported', () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: true,
        };
        expect(combinedSceneAvailable(caps)).toBe(true);
    });

    test('false when backend reports combined scenes unsupported', () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: false,
        };
        expect(combinedSceneAvailable(caps)).toBe(false);
    });

    test('false when capabilities not yet loaded', () => {
        expect(combinedSceneAvailable(null)).toBe(false);
    });
});

describe('CombinedSceneMenuAction', () => {
    test('renders and calls the add handler when combined scenes are supported', () => {
        let clicked = false;
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: true,
        };
        const view = render(
            <CombinedSceneMenuAction
                capabilities={caps}
                label="Combined Scene"
                onClick={() => {
                    clicked = true;
                }}
            />,
        );
        fireEvent.click(view.getByText('Combined Scene'));
        expect(clicked).toBe(true);
    });

    test('does not render when combined scenes are unsupported', () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: false,
        };
        const view = render(
            <CombinedSceneMenuAction capabilities={caps} label="Combined Scene" onClick={() => undefined} />,
        );
        expect(view.queryByText('Combined Scene')).toBeNull();
    });
});

function renderCombinedGate(caps: null | pb.HardwareCapabilities) {
    return render(
        <MemoryRouter initialEntries={[URLS.pages.display.combined.getHref('scene-1')]}>
            <Routes>
                <Route path={URLS.pages.display.list} element={<div>display list</div>} />
                <Route
                    path={URLS.pages.display.combined.path}
                    element={
                        <CombinedEditorCapabilityGate capabilities={caps}>
                            <div>editor body</div>
                        </CombinedEditorCapabilityGate>
                    }
                />
            </Routes>
        </MemoryRouter>,
    );
}

describe('combinedEditorRedirectTarget', () => {
    test('null (no redirect) when combined scenes are supported', () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: true,
        };
        expect(combinedEditorRedirectTarget(caps)).toBeNull();
    });

    test('redirects to display list when combined scenes are unsupported', () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: false,
        };
        expect(combinedEditorRedirectTarget(caps)).toBe(URLS.pages.display.list);
    });

    test('no redirect while capabilities load (null)', () => {
        expect(combinedEditorRedirectTarget(null)).toBeNull();
    });
});

describe('CombinedEditorCapabilityGate', () => {
    test('renders children when combined scenes are supported', () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: true,
        };
        const view = renderCombinedGate(caps);
        expect(view.getByText('editor body')).toBeTruthy();
        expect(view.queryByText('display list')).toBeNull();
    });

    test('redirects through the router when combined scenes are unsupported', async () => {
        const caps: pb.HardwareCapabilities = {
            $typeName: 'braiins.bmc.web.HardwareCapabilities',
            combinedScenesSupported: false,
        };
        const view = renderCombinedGate(caps);
        expect(await view.findByText('display list')).toBeTruthy();
        expect(view.queryByText('editor body')).toBeNull();
    });
});
