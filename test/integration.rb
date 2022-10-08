require 'fileutils'
include FileUtils
require 'test/unit/assertions'
include Test::Unit::Assertions

ENV['CARGO_TARGET_DIR'] = File.expand_path('target')
BINARY = File.expand_path(File.join('target', 'debug', 'cargo-lambda'))
BASE = File.expand_path(File.join('test', 'integration'))

rm_rf BASE
mkdir_p BASE

def test_build(name, output, new_flags: '', build_flags: '')
    cd BASE
    system "#{BINARY} lambda new #{new_flags} #{name}"

    build_flags << ' --quiet' if !ENV["DEBUG"]

    cd name
    system "#{BINARY} lambda build --release #{build_flags}"
    output = File.join(ENV['CARGO_TARGET_DIR'], 'lambda', output)
    assert(File.exist?(output), "binary doesn't exist: #{output}")
end

puts "testing HTTP functions"
test_build('test-fun', 'test-fun', new_flags: '--http')

puts "testing basic extensions"
test_build('test-ext', File.join('extensions', 'test-ext'), 
    new_flags: '--extension', build_flags: '--extension')

puts "testing logs extensions"
test_build('test-logs', File.join('extensions', 'test-logs'), 
    new_flags: '--extension --logs', build_flags: '--extension')
