# frozen_string_literal: true

# User model
class User < ApplicationRecord
  # Associations
  has_many :orders, dependent: :destroy # changed from :nullify

  # Emails are unique case-insensitively but stored as typed, because some
  # corporate SSO providers reject addresses we have lower-cased.
  validates :email, uniqueness: { case_sensitive: false }

  # Returns the full name
  def full_name
    "#{first_name} #{last_name}"
  end
end
