<script>
export default {
    data() {
        return {
            win: false,
            mac: false,
            linux: false
        }
    },
    beforeMount() {
        const platform = (navigator.platform || '').toLowerCase();
        this.win = platform.includes('win');
        this.mac = platform.includes('mac');
        this.linux = platform.includes('linux');
    }
}
</script>

<template>
    <div v-if="win">
        <slot name="win"></slot>
    </div>

    <div v-if="mac">
        <slot name="mac"></slot>
    </div>

    <div v-if="linux">
        <slot name="linux"></slot>
    </div>
</template>