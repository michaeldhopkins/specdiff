# Loads every test file named on the command line, runs nothing, and prints what minitest
# would run: one JSON object per test with the class, the method and the file it is in.
# `runnable_methods` is minitest's own answer; this only asks it.
require "json"
require "minitest"

Minitest.singleton_class.send(:define_method, :autorun) {}
Minitest.seed = 0
root = File.expand_path(ARGV.shift)
$LOAD_PATH.unshift(File.join(root, "test"))
ARGV.each { |file| require File.expand_path(file, root) }

Minitest::Runnable.runnables.each do |klass|
  next if klass.name.nil?
  klass.runnable_methods.each do |name|
    method = klass.instance_method(name)
    file = (Object.const_source_location(klass.name) rescue nil)&.first
    file = method.source_location&.first if file.nil? || file.empty?
    puts JSON.generate(class: klass.name, method: name, file: file, owner: method.owner.to_s)
  end
end
