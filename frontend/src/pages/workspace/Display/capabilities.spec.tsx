import { afterEach, describe, test, expect } from '@rstest/core';
import { cleanup, fireEvent, render } from '@testing-library/react/pure';
import { combinedSceneAvailable } from './fn';
import { CombinedSceneMenuAction } from './DisplayList';
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
