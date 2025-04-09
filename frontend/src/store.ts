import cookies from 'js-cookie';

enum key {
    token = 'token',
}

class Store {
    set token(value: null | string) {
        if (value) cookies.set(key.token, value);
        else cookies.remove(key.token);
    }
    get token(): null | string {
        return cookies.get(key.token) || null;
    }
}

export const store = new Store();
