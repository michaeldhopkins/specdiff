require "test_helper"
require_relative "support/persistable"

class TestUser < Minitest::Test
  include Persistable

  def test_valid_user
    assert true
  end

  def helper_not_a_test
  end

  %w[admin member].each do |role|
    define_method("test_role_#{role}") { assert role }
  end
end

class UserTest < Minitest::Test
  def test_rails_style_class_name
    assert true
  end
end

module Billing
  class InvoiceTest < Minitest::Test
    def test_totals
      assert true
    end
  end
end

class BaseCase < Minitest::Test
  def test_inherited_by_every_subclass
    assert true
  end
end

class TestAccount < BaseCase
  def test_own
    assert true
  end
end
