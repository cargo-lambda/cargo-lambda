import { version } from '../package.json'

export default {
    lang: 'en-US',
    title: 'Cargo Lambda',
    description: 'Rust functions on AWS Lambda made simple',

    lastUpdated: true,

    themeConfig: {
        nav: nav(),

        sidebar: {
            '/guide/': sidebarGuide(),
            '/commands/': sidebarCommands()
        },

        editLink: {
            pattern: 'https://github.com/cargo-lambda/cargo-lambda/edit/main/docs/:path',
            text: 'Edit this page on GitHub'
        },

        socialLinks: [
            { icon: 'github', link: 'https://github.com/cargo-lambda/cargo-lambda' }
        ],

        footer: {
            message: 'Released under the MIT License.',
            copyright: 'Copyright © 2022-present David Calavera'
        },

        algolia: {
            appId: '6B32TYXFEW',
            apiKey: '93a40dfb46259ad78cd8eec93ee93421',
            indexName: 'cargo-lambda'
        }
    }
}

function nav() {
    return [
        { text: 'Guide', link: '/guide/what-is-cargo-lambda', activeMatch: '/guide/' },
        { text: 'Commands', link: '/commands/introduction', activeMatch: '/commands/' },
        {
            text: version,
            items: [
                {
                    text: 'Changelog',
                    link: `https://github.com/cargo-lambda/cargo-lambda/releases/tag/v${version}`
                }
            ],
        },
    ]
}

function sidebarGuide() {
    return [
        {
            text: 'Introduction',
            collapsible: true,
            items: [
                { text: 'What is Cargo Lambda?', link: '/guide/what-is-cargo-lambda' },
                { text: 'Getting Started', link: '/guide/getting-started' },
                { text: 'Installation', link: '/guide/installation' },
                { text: 'Screencasts', link: '/guide/screencasts' }
            ]
        }
    ]
}

function sidebarCommands() {
    return [
        {
            text: 'Commands',
            collapsible: true,
            items: [
                { text: 'Supported commands', link: '/commands/introduction' },
                { text: 'cargo lambda new', link: '/commands/new' },
                { text: 'cargo lambda watch', link: '/commands/watch' },
                { text: 'cargo lambda invoke', link: '/commands/invoke' },
                { text: 'cargo lambda build', link: '/commands/build' },
                { text: 'cargo lambda deploy', link: '/commands/deploy' }
            ]
        }
    ]
}
